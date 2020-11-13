use crate::node::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Remap {
    pub from: BucketId,
    pub into: BucketId,
    /// Into what gamemode to convert.
    pub gamemode: Gamemode,
    /// The default unit to advance when no patterns match.
    pub default_unit: f64,
    /// Similar to `Rekey::weight_curve`.
    pub weight_curve: Vec<(f32, f32)>,
    /// The different prioritized patterns to attempt to apply.
    pub patterns: Vec<Pattern>,
}
impl Default for Remap {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            gamemode: Gamemode::PumpSingle,
            default_unit: 1.,
            weight_curve: vec![(0., 1.), (0.4, 10.), (0.8, 200.), (1.4, 300.)],
            patterns: vec![],
        }
    }
}

impl Node for Remap {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                let notes = remap(sm, self)?;
                sm.notes = notes;
                sm.gamemode = self.gamemode;
            }
            store.put(&self.into, mem::replace(list, default()));
            Ok(())
        })
    }
    fn buckets_mut<'a>(&'a mut self) -> BucketIter<'a> {
        Box::new(
            iter::once((BucketKind::Input, &mut self.from))
                .chain(iter::once((BucketKind::Output, &mut self.into))),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    dist: f64,
    keys: f64,
    unit: f64,
    notes: Vec<(f64, i32)>,
}
impl Default for Pattern {
    fn default() -> Self {
        Self {
            dist: 1.,
            keys: 1.,
            unit: 0.,
            notes: vec![(1., 0)],
        }
    }
}

/// Create entirely new notes, basing the amount of notes per mapping unit on the previous amount
/// of notes on that mapping unit.
fn remap(sm: &mut Simfile, conf: &Remap) -> Result<Vec<Note>> {
    use crate::node::rekey::KeyAlloc;

    let out_keycount = conf.gamemode.key_count() as usize;
    let mut out_notes = Vec::new();

    let mut beats = sm.iter_beats();
    let mut last_beat = BeatPos::from(0.);
    let mut rng = simfile_rng(sm, "remap");
    let mut key_alloc = KeyAlloc::new(&conf.weight_curve, out_keycount);
    let mut tmp_choose_buf = Vec::with_capacity(out_keycount);
    let mut chosen_buf = Vec::with_capacity(out_keycount);
    let mut to_time = sm.beat_to_time();
    trace!("remapping...");
    while !beats.is_empty() {
        let mut pattern = None;
        for pat in conf.patterns.iter() {
            let unit = if pat.unit > 0. {
                pat.unit
            } else {
                conf.default_unit
            };
            let next_beat = last_beat + BeatPos::from(unit);
            let mut tmp_beats = beats.clone();
            let mut simultaneous_sum = 0;
            let mut beat_count = 0;
            for beat in &mut tmp_beats {
                if beat.pos >= next_beat {
                    break;
                }
                let heads = beat.count_heads(&sm.notes);
                if heads > 0 {
                    simultaneous_sum += heads;
                    beat_count += 1;
                }
            }
            if beat_count > 0 {
                let dist_avg = unit / (beat_count + 1) as f64;
                let simultaneous_avg = simultaneous_sum as f64 / beat_count as f64;
                if dist_avg <= pat.dist && simultaneous_avg >= pat.keys {
                    //Use this pattern
                    trace!("  matched pattern {:?} on beat {}", pat, last_beat);
                    pattern = Some((pat, unit));
                    beats = tmp_beats;
                    break;
                }
            }
        }
        match pattern {
            Some((pat, unit)) => {
                //Generate this pattern
                chosen_buf.clear();
                let mut last_rel_beat = 0.;
                tmp_choose_buf.clear();
                tmp_choose_buf.extend(0..out_keycount);
                for &(rel_beat, key_placeholder) in pat.notes.iter() {
                    //Sanitize pattern
                    ensure!(key_placeholder >= 0, "pattern key cannot be negative");
                    ensure!(
                        rel_beat >= last_rel_beat,
                        "pattern beats must increase monotonically"
                    );
                    if rel_beat > last_rel_beat {
                        tmp_choose_buf.clear();
                        tmp_choose_buf.extend(0..out_keycount);
                    }
                    last_rel_beat = rel_beat;
                    ensure!(
                        rel_beat <= unit,
                        "pattern beats cannot go beyond the pattern unit"
                    );
                    let key_placeholder = key_placeholder as usize;

                    //Get the absolute beat and time
                    let beat = last_beat + BeatPos::from(rel_beat);
                    let time = to_time.beat_to_time(beat);

                    //Get the key
                    let key = if key_placeholder < chosen_buf.len() {
                        //Reuse an allocated key
                        chosen_buf[key_placeholder]
                    } else if key_placeholder == chosen_buf.len() {
                        //Allocate a new key
                        let (pos, out_key) = key_alloc.alloc_idx(&tmp_choose_buf, time, &mut rng).ok_or_else(|| anyhow!("pattern key placeholder {} allocated too many keys on the same beat for keycount ({})", key_placeholder, out_keycount))?;
                        tmp_choose_buf.swap_remove(pos);
                        chosen_buf.push(out_key);
                        trace!(
                            "    allocated key {} for placeholder {}",
                            out_key,
                            key_placeholder
                        );
                        out_key
                    } else {
                        bail!(
                            "pattern key placeholder {} skips indices (next key placeholder would be {})",
                            key_placeholder,
                            chosen_buf.len()
                        )
                    };

                    //Add a note on this beat and key
                    trace!("    placing note at beat {}, key {}", beat, key);
                    key_alloc.touch(key, time);
                    out_notes.push(Note {
                        beat,
                        key: key as i32,
                        kind: Note::KIND_HIT,
                    });
                }
                last_beat += BeatPos::from(unit);
            }
            None => {
                //No patterns found, maybe this is an empty part of the song
                //Advance by `default_unit` beats
                let default_unit = BeatPos::from(conf.default_unit);
                last_beat = last_beat.floor(default_unit) + default_unit;
                while let Some(beat) = beats.peek() {
                    if beat.pos >= last_beat {
                        break;
                    } else {
                        beats.next();
                    }
                }
                trace!("  no pattern matched, advanced to beat {}", last_beat);
            }
        }
    }
    Ok(out_notes)
}
