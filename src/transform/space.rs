//! Make a minimum space between notes by removing higher-divisor notes.

use crate::transform::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Space {
    pub from: BucketId,
    pub into: BucketId,
    pub min_dist: MinDist,
}
impl Default for Space {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            min_dist: MinDist::Bpm(120.),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MinDist {
    Bpm(f64),
    Beats(f64),
}
impl Default for MinDist {
    fn default() -> Self {
        Self::Beats(1.)
    }
}

impl Transform for Space {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            for sm in list.iter_mut() {
                make_space(sm, self)?;
            }
            store.put(&self.into, list);
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

fn make_space(sm: &mut Simfile, conf: &Space) -> Result<()> {
    // To prevent any recognizable patterns from forming
    let mut rng = simfile_rng(sm, "space");
    // Cache note times, because notes will be randomly accessed
    let note_times = {
        let mut to_time = ToTime::new(sm);
        sm.notes
            .iter()
            .map(|note| to_time.beat_to_time(note.beat))
            .collect::<Vec<_>>()
    };
    // Minimum distance between notes
    let min_limit_secs;
    let secs_func;
    let min_limit_beats;
    let beat_func;
    let are_far_enough: &dyn Fn(&[Note], usize, usize) -> bool = match conf.min_dist {
        MinDist::Bpm(bpm) => {
            min_limit_secs = 60. / bpm - 0.010;
            trace!(
                "    removing notes in order to make a minimum distance of {}s",
                min_limit_secs,
            );
            secs_func = |_notes: &[Note], a: usize, b: usize| {
                note_times[b] - note_times[a] >= min_limit_secs
            };
            &secs_func
        }
        MinDist::Beats(beats) => {
            min_limit_beats = BeatPos::from(beats);
            trace!(
                "    removing notes in order to make a minimum distance of {} beats",
                min_limit_beats,
            );
            beat_func = |notes: &[Note], a: usize, b: usize| {
                notes[b].beat - notes[a].beat >= min_limit_beats
            };
            &beat_func
        }
    };
    //Create an array of references to notes, sorted from most removable to least removable
    let mut note_refs = (0..sm.notes.len())
        .filter(|&idx| !sm.notes[idx].is_tail())
        .collect::<Vec<_>>();
    note_refs.sort_by_cached_key(|&idx| {
        ((64 - sm.notes[idx].beat.denominator() as u32) << (32 - 6))
            | ((rng.gen::<u32>() << 6) >> 6)
    });
    // Remove any notes that have neighbors that are too close
    for &note_idx in note_refs.iter() {
        let this_beat = sm.notes[note_idx].beat;
        let mut keep = true;

        //Check forward gap
        if let Some(indices_to_next_note) = sm.notes[note_idx + 1..]
            .iter()
            .position(|note| !note.is_tail() && note.key >= 0 && note.beat > this_beat)
        {
            let next_note = note_idx + 1 + indices_to_next_note;
            keep = are_far_enough(&sm.notes, note_idx, next_note);
        }

        //Check backward gap
        if keep {
            if let Some(indices_to_prev_note) = sm.notes[..note_idx]
                .iter()
                .rev()
                .position(|note| !note.is_tail() && note.key >= 0 && note.beat < this_beat)
            {
                let prev_note = note_idx - 1 - indices_to_prev_note;
                keep = are_far_enough(&sm.notes, prev_note, note_idx);
            }
        }

        //Remove if too close
        if !keep {
            //If removing a head, also remove its tail
            if sm.notes[note_idx].is_head() {
                let head_key = sm.notes[note_idx].key;
                for next_note in sm.notes[note_idx + 1..].iter_mut() {
                    if next_note.is_tail() && next_note.key == head_key {
                        next_note.key = -1;
                        break;
                    }
                }
            }
            //Mark this note for removal
            sm.notes[note_idx].key = -1;
        }
    }
    //Actually remove notes
    sm.notes.retain(|note| note.key >= 0);
    //*
    //Sanity check
    let mut to_time = ToTime::new(sm);
    let mut last_time = 0.;
    let notes_without_tails = sm
        .notes
        .iter()
        .filter(|note| !note.is_tail())
        .cloned()
        .collect::<Vec<_>>();
    for (idx, note) in notes_without_tails.iter().enumerate() {
        let time = to_time.beat_to_time(note.beat);
        if idx > 0 {
            let prev = &notes_without_tails[idx - 1];
            match conf.min_dist {
                MinDist::Bpm(bpm) => {
                    let min_dist = 60. / bpm;
                    let dist = (time - last_time).abs();
                    ensure!(
                        note.beat == prev.beat || dist >= min_dist,
                        "sanity check failed: notes at beats {} and {} are only {}s apart (should be at least {}s apart)",
                        prev.beat,
                        note.beat,
                        dist,
                        min_dist,
                    );
                }
                MinDist::Beats(beats) => {
                    let min_dist_beats = BeatPos::from(beats);
                    ensure!(
                        note.beat == prev.beat || note.beat - prev.beat >= min_dist_beats,
                        "sanity check failed: notes at beats {} and {} are only {} beats apart (should be at least {} beats apart)",
                        prev.beat,
                        note.beat,
                        note.beat-prev.beat,
                        min_dist_beats,
                    );
                }
            }
        }
        last_time = time;
    }
    // */
    Ok(())
}
