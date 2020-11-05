use crate::transform::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipe {
    pub from: BucketId,
    pub into: BucketId,
    /// Whether to merge all input lists into one superlist.
    pub merge: bool,
}

impl Default for Pipe {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            merge: false,
        }
    }
}

impl Transform for Pipe {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        if self.merge {
            let mut merged = Vec::new();
            store.get(&self.from, |_, mut list| {
                if merged.is_empty() {
                    merged = list;
                } else {
                    merged.append(&mut list);
                }
                Ok(())
            })?;
            store.put(&self.into, merged);
            Ok(())
        } else {
            store.get(&self.from, |store, list| {
                store.put(&self.into, list);
                Ok(())
            })
        }
    }
    fn buckets_mut<'a>(&'a mut self) -> BucketIter<'a> {
        Box::new(
            iter::once((BucketKind::Input, &mut self.from))
                .chain(iter::once((BucketKind::Output, &mut self.into))),
        )
    }
}
