//! Rate a simfile's difficulty.
//!
//! Perhaps one day the groove meter and whatnot could be updated, but for now it's just
//! in-practice BPM estimation.

use crate::transform::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Rate {
    pub from: BucketId,
    pub into: BucketId,
    /// The method to use to produce a numerical rating.
    pub method: RateMethod,
    /// Apply a linear mapping to the output numerical difficulty.
    /// This field represents two ranges, one for input and one for output, and the difficulty scale
    /// is modified based on both.
    pub scale: [f64; 4],
    /// Whether to update the song numerical difficulty meter from the output of the rating.
    pub set_meter: bool,
    /// Whether to update the song qualitative difficulty from the numerical difficulty.
    ///
    /// If this array is empty, the difficulty is not updated.
    /// Entries for this array are (numerical, qualitative) entries.
    /// The numerically closest entry to the computed difficulty is used.
    ///
    /// These numbers might require manual tuning to adjust for the scales used by different rating
    /// methods.
    pub set_diff: Vec<(f64, Difficulty)>,
}
impl Default for Rate {
    fn default() -> Self {
        use crate::simfile::Difficulty::*;
        Self {
            from: default(),
            into: default(),
            method: default(),
            scale: [0., 1., 0., 1.],
            set_meter: true,
            set_diff: vec![
                (1., Beginner),
                (2.5, Easy),
                (5., Medium),
                (7.5, Hard),
                (10., Challenge),
                (15., Edit),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RateMethod {
    EffectiveBpm { exponent: f64 },
}
impl Default for RateMethod {
    fn default() -> Self {
        Self::EffectiveBpm { exponent: 2. }
    }
}

impl Transform for Rate {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            for sm in list.iter_mut() {
                rate(self, sm)?;
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

fn rate(conf: &Rate, sm: &mut Simfile) -> Result<()> {
    let computed = match &conf.method {
        RateMethod::EffectiveBpm { exponent } => {
            // Get the in-practice BPM of this simfile.
            get_practical_bpm(*exponent, sm)
        }
    };
    let scaled = {
        let [in_min, in_max, out_min, out_max] = conf.scale;
        linear_map(in_min, in_max, out_min, out_max)(computed)
    };
    if conf.set_meter {
        sm.difficulty_num = scaled;
    }
    if let Some((_num, diff)) = conf
        .set_diff
        .iter()
        .min_by_key(|(num, _diff)| SortableFloat((*num - scaled).abs()))
    {
        sm.difficulty = *diff;
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
