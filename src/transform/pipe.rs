use crate::transform::prelude::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipe {
    pub from: BucketId,
    pub into: BucketId,
}
impl Transform for Pipe {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
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
