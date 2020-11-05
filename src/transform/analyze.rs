//! Analyzes a simfile and updates its metadata.
//!
//! Perhaps one day the groove meter and whatnot could be updated, but for now it's just
//! in-practice BPM estimation.

use crate::transform::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Analyze {
    pub from: BucketId,
    pub into: BucketId,
    pub difficulty: AnalyzeDifficulty,
}
impl Default for Analyze {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            difficulty: default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnalyzeDifficulty {
    Dont,
    PracticalBpm { exponent: f64 },
}
impl Default for AnalyzeDifficulty {
    fn default() -> Self {
        Self::PracticalBpm { exponent: 2. }
    }
}

impl Transform for Analyze {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            for sm in list.iter_mut() {
                analyze(self, sm)?;
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

fn analyze(conf: &Analyze, sm: &mut Simfile) -> Result<()> {
    match &conf.difficulty {
        AnalyzeDifficulty::PracticalBpm { exponent } => {
            // Get the in-practice BPM of this simfile.
            sm.difficulty_num = get_practical_bpm(*exponent, sm);
        }
        AnalyzeDifficulty::Dont => {}
    }
    Ok(())
}

fn get_practical_bpm(exp: f64, sm: &mut Simfile) -> f64 {
    let exp = exp as f32;
    let mut last_time = None;
    let mut to_time = sm.beat_to_time();
    let mut total_freq = 0.;
    let mut total_gaps = 0;
    for (beat, _, _) in sm.iter_beats() {
        let time = to_time.beat_to_time(beat);
        if let Some(last_time) = last_time {
            let gap = (time - last_time) as f32;
            let freq = 1. / gap;
            total_freq += freq.powf(exp);
            total_gaps += 1;
        }
        last_time = Some(time);
    }
    total_freq = (total_freq / total_gaps as f32).powf(1. / exp);
    (total_freq * 60.) as f64
}
