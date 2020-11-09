use crate::node::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Remap {
    pub from: BucketId,
    pub into: BucketId,
    /// Into what gamemode to convert.
    pub gamemode: Gamemode,
    /// If the input keycount is the same as the output keycount, avoid key changes.
    pub avoid_shuffle: bool,
    /// Weighting options to prevent too many jacks (quick notes on the same key).
    pub weight_curve: Vec<(f32, f32)>,
}
impl Default for Remap {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            gamemode: Gamemode::PumpSingle,
            avoid_shuffle: true,
            weight_curve: vec![(0., 1.), (0.4, 10.), (0.8, 200.), (1.4, 300.)],
        }
    }
}

impl Node for Remap {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                convert(sm, self)?;
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

fn convert(sm: &mut Simfile, conf: &Remap) -> Result<()> {
    // Map the amount of time since the key was last active to a choose weight.
    // Try to map higher times to more weight, but not too much
    let inactive_time_to_weight = {
        let mut points = Vec::with_capacity(conf.weight_curve.len().saturating_sub(1));
        for i in 0..conf.weight_curve.len().saturating_sub(1) {
            let (this_x, this_y) = conf.weight_curve[i];
            let (next_x, next_y) = conf.weight_curve[i + 1];
            let m = (next_y - this_y) / (next_x - this_x);
            points.push((next_x, m, -this_x * m + this_y));
        }
        let default_val = conf.weight_curve.last().map(|(_x, y)| *y).unwrap_or(1.);
        move |time: f32| {
            for &(up_to, m, c) in points.iter() {
                if time <= up_to {
                    return m * time + c;
                }
            }
            default_val
        }
    };

    //Keycounts
    let in_keycount = sm.gamemode.key_count() as usize;
    let out_keycount = conf.gamemode.key_count() as usize;
    trace!("    converting {}K to {}K", in_keycount, out_keycount);
    ensure!(in_keycount > 0, "cannot convert 0-key map");
    ensure!(out_keycount > 0, "cannot convert to 0-key map");

    //Do nothing if there is nothing to do
    if conf.avoid_shuffle && in_keycount == out_keycount {
        sm.gamemode = conf.gamemode;
        return Ok(());
    }

    //Detach note buffer for lifetiming purposes
    let mut notes = mem::replace(&mut sm.notes, Vec::new());
    //To randomize key mappings
    let mut rng = simfile_rng(sm, "convert");
    //Beat -> time
    let mut to_time = ToTime::new(sm);

    //Holds the last active time (ie. when was the last time a key was down) for each outkey.
    let mut last_active_times = vec![to_time.beat_to_time(BeatPos::from(0.)); out_keycount];
    //Holds which outkeys are locked.
    //If the inner option is `Some`, that outkey should be unlocked after that beat passes.
    let mut locked_outkeys = vec![None; out_keycount];
    //If a tail occurs at the given inkey, unlock the stored outkey.
    let mut unlock_by_tails = vec![0; in_keycount];
    //Auxiliary buffer to choose weighted outkeys
    let mut choose_tmp_buf = Vec::with_capacity(out_keycount);

    for note in notes.iter_mut() {
        let note_time = to_time.beat_to_time(note.beat);
        //Unlock any auto-unlocking keys
        for locked in locked_outkeys.iter_mut() {
            if let Some(Some(unlock_after)) = *locked {
                if note.beat > unlock_after {
                    *locked = None;
                }
            }
        }
        //Map key
        let mapped_key = if note.is_tail() {
            let out_key = unlock_by_tails[note.key as usize];
            locked_outkeys[out_key] = None;
            last_active_times[out_key] = note_time;
            out_key as i32
        } else {
            //Choose an outkey using randomness and weights
            choose_tmp_buf.clear();
            choose_tmp_buf.extend(
                locked_outkeys
                    .iter()
                    .enumerate()
                    .filter(|(_i, locked)| locked.is_none())
                    .map(|(i, _locked)| i),
            );
            let mapped = choose_tmp_buf
                .choose_weighted(&mut rng, |&out_key| {
                    let time = (note_time - last_active_times[out_key]) as f32;
                    let weight = inactive_time_to_weight(time);
                    weight
                })
                .ok();
            match mapped {
                Some(&out_key) => {
                    if note.is_head() {
                        locked_outkeys[out_key] = Some(None);
                        unlock_by_tails[note.key as usize] = out_key;
                    } else {
                        locked_outkeys[out_key] = Some(Some(note.beat));
                    }
                    last_active_times[out_key] = note_time;
                    out_key as i32
                }
                None => {
                    //All output keys are locked
                    -1
                }
            }
        };
        note.key = mapped_key;
    }
    notes.retain(|note| note.key >= 0);
    //Finally, modify simfile
    sm.notes = notes;
    sm.gamemode = conf.gamemode;
    Ok(())
}
