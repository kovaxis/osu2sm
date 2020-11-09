//! Only one of each difficulty can be output for each gamemode, effectively limiting the
//! amount of charts per song per gamemode to 6, tops.
//! Damn good design.

use crate::node::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Select {
    pub from: BucketId,
    pub into: BucketId,
    /// Group input simfiles by music and gamemode before trimming difficulties.
    pub merge: bool,
    /// The maximum amount of simfiles to select per list.
    /// Having a value larger than the length of `diff_names` makes no effect.
    pub max: usize,
    /// Define the criteria to use to evict difficulties.
    pub prefer: PreferDiff,
    /// The minimum required distance between difficulties in order to keep them.
    /// This minimum is applied first, before any difficulty trimming.
    pub dedup_dist: f64,
    /// When deduplicating, which difficulty in the overlap range to choose.
    /// A value of `0` means "the easiest", while a value of `1` means "the hardest".
    pub dedup_bias: f64,
    /// Which difficulties can be present in the output, and how many times each.
    /// Difficulties should be sorted according to user preference.
    ///
    /// Defaults to the entire range of difficulties (`Beginner` - `Challenge`, `Edit`).
    pub diff_names: Vec<Difficulty>,
}
impl Default for Select {
    fn default() -> Self {
        use crate::simfile::Difficulty::*;
        Self {
            from: default(),
            into: default(),
            merge: true,
            max: 6,
            diff_names: vec![Beginner, Easy, Medium, Hard, Challenge, Edit],
            prefer: default(),
            dedup_dist: 0.,
            dedup_bias: 0.5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PreferDiff {
    /// Maximize the range of available difficulties.
    ///
    /// This mode will always keep the extreme difficulties, except when truncating to 1 difficulty.
    /// In this special case, the most central difficulty will be kept.
    Spread,
    /// Minimize the distance to the given numerical difficulty spread.
    /// Note that more than this amount of difficulties might be outputted.
    ///
    /// If `min` and `max` are given, the difficulty spread dataset will be stretched so
    /// that `min` and `max` will match the minimum and maximum input difficulties,
    /// respectively.
    ClosestMatch {
        to: Vec<f64>,
        #[serde(default)]
        min: f64,
        #[serde(default)]
        max: f64,
    },
    /// Prefer the minimum difficulties.
    Easier,
    /// Prefer the maximum difficulties.
    Harder,
}
impl Default for PreferDiff {
    fn default() -> Self {
        Self::Spread
    }
}
impl PreferDiff {
    fn evict<T>(
        &self,
        diffs: &mut Vec<T>,
        as_diff: impl Fn(&T) -> f64,
        truncate_to: usize,
    ) -> Result<()> {
        let match_dataset = |diffs: &mut Vec<T>, dataset: &[f64]| {
            while diffs.len() > truncate_to {
                //Find the largest minimum distance
                let (to_remove, _) = diffs
                    .iter()
                    .enumerate()
                    .max_by_key(|&(_idx, diff)| {
                        let diff = as_diff(diff);
                        let next_datapoint = dataset
                            .iter()
                            .position(|&data| data >= diff)
                            .unwrap_or(dataset.len());
                        //Gap before
                        let prev_gap = if next_datapoint > 0 {
                            diff - dataset[next_datapoint - 1]
                        } else {
                            f64::INFINITY
                        };
                        //Gap after
                        let next_gap = if next_datapoint < dataset.len() {
                            dataset[next_datapoint] - diff
                        } else {
                            f64::INFINITY
                        };
                        //Find the smallest gap
                        SortableFloat(prev_gap.min(next_gap))
                    })
                    .unwrap();
                //Remove this chart :(
                diffs.remove(to_remove);
            }
        };
        if diffs.is_empty() {
            return Ok(());
        }
        match self {
            PreferDiff::Spread => {
                let min = as_diff(diffs.first().unwrap());
                let range = as_diff(diffs.last().unwrap()) - min;
                if truncate_to == 1 {
                    match_dataset(diffs, &[min + range / 2.]);
                } else {
                    let max_idx = (truncate_to - 1) as f64;
                    let dataset = (0..truncate_to)
                        .map(|idx| min + range * (idx as f64 / max_idx))
                        .collect::<Vec<_>>();
                    match_dataset(diffs, &dataset);
                }
            }
            PreferDiff::ClosestMatch {
                to: dataset,
                min,
                max,
            } => {
                if *min == *max {
                    match_dataset(diffs, dataset);
                } else {
                    let (out_min, out_max) = (
                        as_diff(diffs.first().unwrap()),
                        as_diff(diffs.last().unwrap()),
                    );
                    let map = linear_map(*min, *max, out_min, out_max);
                    let stretched = dataset.iter().map(|&diff| map(diff)).collect::<Vec<_>>();
                    match_dataset(diffs, &stretched);
                }
            }
            PreferDiff::Easier => {
                diffs.truncate(truncate_to);
            }
            PreferDiff::Harder => {
                if truncate_to < diffs.len() {
                    diffs.drain(..diffs.len() - truncate_to);
                }
            }
        }
        Ok(())
    }
}

impl Node for Select {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        let process_list = |store: &mut SimfileStore, mut list: Vec<Box<Simfile>>| -> Result<()> {
            trim_difficulties(self, &mut list)?;
            store.put(&self.into, list);
            Ok(())
        };
        if self.merge {
            let mut by_music_gamemode: HashMap<(PathBuf, Gamemode), Vec<Box<Simfile>>> = default();
            store.get_each(&self.from, |_, sm| {
                let list = by_music_gamemode
                    .entry((sm.music.clone().unwrap_or_default(), sm.gamemode))
                    .or_default();
                list.push(sm);
                Ok(())
            })?;
            for (_, list) in by_music_gamemode {
                process_list(store, list)?;
            }
            Ok(())
        } else {
            store.get(&self.from, |store, list| {
                process_list(store, mem::replace(list, default()))
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

/// There seems to be a max of 6 difficulties, so use them wisely and sort them.
pub fn trim_difficulties(conf: &Select, simfiles: &mut Vec<Box<Simfile>>) -> Result<()> {
    //Exit early on the degenerate case, because weird stuff happens in these edge cases
    if conf.diff_names.is_empty() || conf.max <= 0 {
        simfiles.clear();
        return Ok(());
    }

    //Make sure some rating system was used
    ensure!(
        simfiles.iter().all(|sm| sm.difficulty_num.is_finite()),
        "cannot fix simfiles without a difficulty rating (use the `Rate` node before `SimfileFix`)"
    );

    //Create an auxiliary vec holding chart indices and difficulties
    let mut order = simfiles
        .iter()
        .map(|sm| sm.difficulty_num)
        .enumerate()
        .collect::<Vec<_>>();
    trace!("    raw difficulties: {:?}", order);

    //Sort by difficulty
    order.sort_by_key(|(_, d)| SortableFloat(*d));
    trace!("    sorted difficulties: {:?}", order);

    //Remove difficulties if they are too close
    //Note that `<` is used, so that `min_dist == 0` implies that no removals are made.
    {
        let mut idx = 0;
        macro_rules! diff_at {
            ($idx:expr) => {
                order[$idx].1
            };
        }
        while idx < order.len() {
            let bucket_start = idx;
            let bucket_min_diff = diff_at!(idx);
            let mut bucket_max_diff = bucket_min_diff;
            idx += 1;
            while idx < order.len() && diff_at!(idx) - bucket_min_diff < conf.dedup_dist {
                bucket_max_diff = diff_at!(idx);
                idx += 1;
            }
            let mid_diff = bucket_min_diff + (bucket_max_diff - bucket_min_diff) * conf.dedup_bias;
            let mid_idx = (bucket_start..idx)
                .find(|&i| diff_at!(i) >= mid_diff)
                .unwrap();
            for i in bucket_start..idx {
                if i != mid_idx {
                    diff_at!(i) = f64::NAN;
                }
            }
        }
        order.retain(|&(_, diff)| !diff.is_nan());
    }

    //Evict difficulties
    conf.prefer
        .evict(&mut order, |(_, d)| *d, conf.max.min(conf.diff_names.len()))?;
    trace!("    with conflicts resolved: {:?}", order);

    //Reorder charts
    for chart in simfiles.iter_mut() {
        chart.difficulty_num = f64::NAN;
    }
    for (idx, diff) in order.iter() {
        simfiles[*idx].difficulty_num = *diff;
    }
    simfiles.retain(|chart| !chart.difficulty_num.is_nan());
    simfiles.sort_by_key(|chart| SortableFloat(chart.difficulty_num));
    trace!(
        "    final chart difficulties: {:?}",
        simfiles
            .iter()
            .map(|chart| chart.difficulty_num)
            .collect::<Vec<_>>()
    );

    //Convert difficulties into difficulty indices
    let mut difficulties = simfiles
        .iter()
        .map(|sm| {
            conf.diff_names
                .iter()
                .position(|diff| *diff == sm.difficulty)
                .unwrap_or(conf.diff_names.len() - 1) as isize
        })
        .collect::<Vec<_>>();
    trace!("    diff indices: {:?}", difficulties);

    //Resolve conflicts
    loop {
        let mut conflict = None;
        for (i, window) in difficulties.windows(2).enumerate() {
            if window[1] == window[0] {
                //Conflict
                //See which way is the conflict solved faster
                let direction_cost = |idx: usize, dir: isize| {
                    let mut idx = idx as isize;
                    let mut occupied_if = difficulties[idx as usize];
                    let mut cost = 0.;
                    while occupied_if >= 0
                        && occupied_if < conf.diff_names.len() as isize
                        && idx >= 0
                        && idx < difficulties.len() as isize
                    {
                        if (difficulties[idx as usize] - occupied_if) * dir <= 0 {
                            idx += dir;
                            occupied_if += dir;
                            cost += 1.;
                        } else {
                            break;
                        }
                    }
                    if occupied_if < 0 || occupied_if >= conf.diff_names.len() as isize {
                        //Saturated. Max cost
                        9999.
                    } else {
                        cost
                    }
                };
                trace!("    conflict on {} - {}", i, i + 1);
                if direction_cost(i, -1) < direction_cost(i + 1, 1) {
                    //Solve to the left
                    conflict = Some((i, -1));
                } else {
                    //Solve to the right
                    conflict = Some((i + 1, 1));
                }
                break;
            }
        }

        match conflict {
            Some((idx, dir)) => {
                let mut idx = idx as isize;
                trace!("      solving on idx {}, direction {}", idx, dir);
                let mut set_to = difficulties[idx as usize] + dir;
                while idx >= 0
                    && idx < difficulties.len() as isize
                    && (difficulties[idx as usize] - set_to) * dir <= 0
                {
                    set_to = set_to.min(conf.diff_names.len() as isize - 1).max(0);
                    trace!(
                        "      moving difficulties[{}] == {} -> {}",
                        idx,
                        difficulties[idx as usize],
                        set_to
                    );
                    difficulties[idx as usize] = set_to;
                    idx += dir;
                    set_to += dir;
                }
            }
            None => break,
        }
    }
    trace!(
        "    diff indices with conflicts resolved: {:?}",
        difficulties
    );

    //Convert back from difficulty indices to actual difficulties
    for (chart, diff_idx) in simfiles.iter_mut().zip(difficulties) {
        chart.difficulty = conf.diff_names[diff_idx as usize];
        chart.difficulty_num = chart.difficulty_num.round();
    }
    trace!(
        "    final chart difficulties: {:?}",
        simfiles
            .iter()
            .map(|chart| format!("{:?} ({})", chart.difficulty, chart.difficulty_num))
            .collect::<Vec<_>>()
    );

    Ok(())
}
