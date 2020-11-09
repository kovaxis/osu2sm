use crate::node::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Align {
    pub from: BucketId,
    pub into: BucketId,
    pub to: f64,
}
impl Default for Align {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            to: 1.,
        }
    }
}

impl Node for Align {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                align(sm, self)?;
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

fn align(sm: &mut Simfile, conf: &Align) -> Result<()> {
    let align_to = BeatPos::from(conf.to);
    for i in 0..sm.notes.len() {
        let note = &mut sm.notes[i];
        if !note.is_tail() && !note.beat.is_aligned(align_to) {
            let head_key = note.key;
            note.key = -1;
            if note.is_head() {
                //If note is a head, also remove its tail
                for j in i + 1..sm.notes.len() {
                    let note = &mut sm.notes[j];
                    if note.key == head_key && note.is_tail() {
                        note.key = -1;
                        break;
                    }
                }
            }
        }
    }
    sm.notes.retain(|note| note.key >= 0);
    Ok(())
}
