use crate::transform::prelude::*;

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

impl Transform for Align {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            for sm in list.iter_mut() {
                align(sm, self)?;
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

fn align(sm: &mut Simfile, conf: &Align) -> Result<()> {
    let align_to = BeatPos::from(conf.to);
    sm.notes.retain(|note| note.beat.is_aligned(align_to));
    Ok(())
}
