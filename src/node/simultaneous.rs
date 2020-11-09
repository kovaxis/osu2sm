use crate::node::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Simultaneous {
    pub from: BucketId,
    pub into: BucketId,
    /// A value of `-1` indicates "no limit".
    pub max_keys: i32,
}
impl Default for Simultaneous {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            max_keys: -1,
        }
    }
}

impl Node for Simultaneous {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                limit_simultaneous_keys(sm, self)?;
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

fn limit_simultaneous_keys(sm: &mut Simfile, conf: &Simultaneous) -> Result<()> {
    let max_simultaneous = conf.max_keys as usize;
    let key_count = sm.gamemode.key_count() as usize;
    trace!(
        "    limiting max simultaneous keys to {}/{}K",
        max_simultaneous,
        key_count,
    );
    let mut rng = simfile_rng(sm, "simultaneous");
    let mut active_notes = vec![false; key_count];
    let mut beat_notes = Vec::with_capacity(key_count);
    let mut note_idx = 0;
    while note_idx < sm.notes.len() {
        //Go through the notes in this beat
        let cur_beat = sm.notes[note_idx].beat;
        let mut tmp_active_notes = 0;
        beat_notes.clear();
        while note_idx < sm.notes.len() && sm.notes[note_idx].beat == cur_beat {
            //Check out this note
            let note = &mut sm.notes[note_idx];
            if note.is_tail() {
                if active_notes[note.key as usize] {
                    active_notes[note.key as usize] = false;
                } else {
                    note.key = -1;
                }
            } else {
                beat_notes.push(note_idx);
                if note.is_head() {
                    active_notes[note.key as usize] = true;
                } else {
                    tmp_active_notes += 1;
                }
            }
            //Advance to the next note
            note_idx += 1;
        }
        //Determine how many notes to remove
        let total_active_notes =
            active_notes.iter().map(|&b| b as usize).sum::<usize>() + tmp_active_notes;
        let notes_to_remove = total_active_notes.saturating_sub(max_simultaneous);
        //Actually remove notes
        for &rem_note in beat_notes.choose_multiple(&mut rng, notes_to_remove) {
            let note = &mut sm.notes[rem_note];
            if note.is_head() {
                active_notes[note.key as usize] = false;
            }
            note.key = -1;
        }
    }
    //Actually remove notes
    sm.notes.retain(|note| note.key >= 0);
    Ok(())
}
