use crate::transform::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Snap {
    pub from: BucketId,
    pub into: BucketId,
    pub bpm: f64,
}
impl Default for Snap {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            bpm: 120.,
        }
    }
}

impl Transform for Snap {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            for sm in list.iter_mut() {
                snap(sm, self)?;
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

fn snap(sm: &mut Simfile, conf: &Snap) -> Result<()> {
    // Minimum distance between notes
    let min_dist = 60. / conf.bpm - 0.010;
    trace!(
        "    removing notes in order to make a minimum distance of {}s",
        min_dist
    );
    // To prevent any recognizable patterns from forming
    let mut rng = simfile_rng(sm, "snap");
    // Cache note times, because notes will be randomly accessed
    let note_times = {
        let mut to_time = ToTime::new(sm);
        sm.notes
            .iter()
            .map(|note| to_time.beat_to_time(note.beat))
            .collect::<Vec<_>>()
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
        let this_time = note_times[note_idx];
        let mut keep = true;

        //Check forward gap
        if let Some(indices_to_next_note) = sm.notes[note_idx + 1..]
            .iter()
            .position(|note| !note.is_tail() && note.key >= 0 && note.beat > this_beat)
        {
            let next_note = note_idx + 1 + indices_to_next_note;
            let gap = note_times[next_note] - this_time;
            keep = gap >= min_dist;
            trace!(
                "        forward gap from {} - {}: {}s ({})",
                note_idx,
                next_note,
                gap,
                if keep { "keeping" } else { "removing" }
            );
        }

        //Check backward gap
        if keep {
            if let Some(indices_to_prev_note) = sm.notes[..note_idx]
                .iter()
                .rev()
                .position(|note| !note.is_tail() && note.key >= 0 && note.beat < this_beat)
            {
                let prev_note = note_idx - 1 - indices_to_prev_note;
                let gap = this_time - note_times[prev_note];
                keep = gap >= min_dist;
                trace!(
                    "        backward gap from {} - {}: {}s ({})",
                    note_idx,
                    prev_note,
                    gap,
                    if keep { "keeping" } else { "removing" }
                );
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
    /*
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
        last_time = time;
    }
    // */
    Ok(())
}
