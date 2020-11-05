//! Simfiles have several (stupid) limitations.
//!
//! Fix them, ideally before outputting.

use crate::transform::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SimfileFix {
    pub from: BucketId,
    pub into: BucketId,
    /// Which difficulties to output.
    /// Only one of each difficulty can be output for each gamemode, effectively limiting the
    /// amount of charts per song per gamemode to 6, tops.
    /// Damn good design.
    ///
    /// Defaults to the entire range of difficulties (`Beginner` - `Challenge`, `Edit`).
    pub out_difficulties: Vec<Difficulty>,
    /// Holds the difficulty number equivalent to each entry in `out_diffs`.
    /// Used to map meters -> difficulty.
    pub equivalent_meters: Vec<f64>,
    /// Fix the stupid simfile format that doesn't support holds ending and another note starting
    /// at the same time.
    /// Pushes hold tails that are on the same beat and key as another note 1 microbeat backward.
    pub fix_holds: bool,
}
impl Default for SimfileFix {
    fn default() -> Self {
        use crate::simfile::Difficulty::*;
        Self {
            from: default(),
            into: default(),
            out_difficulties: vec![Beginner, Easy, Medium, Hard, Challenge, Edit],
            equivalent_meters: vec![1., 2., 3.5, 5., 6.5, 8.],
            fix_holds: true,
        }
    }
}

impl Transform for SimfileFix {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        let mut by_music_gamemode: HashMap<(PathBuf, Gamemode), Vec<Box<Simfile>>> = default();
        store.get_each(&self.from, |_, sm| {
            let list = by_music_gamemode
                .entry((sm.music.clone().unwrap_or_default(), sm.gamemode))
                .or_default();
            list.push(sm);
            Ok(())
        })?;
        for (_, mut list) in by_music_gamemode {
            spread_difficulties(self, &mut list)?;
            for sm in list.iter_mut() {
                sm.fix_tails()?;
            }
            store.put(&self.into, list);
        }
        Ok(())
    }
    fn buckets_mut<'a>(&'a mut self) -> BucketIter<'a> {
        Box::new(
            iter::once((BucketKind::Input, &mut self.from))
                .chain(iter::once((BucketKind::Output, &mut self.into))),
        )
    }
}

/// There seems to be a max of 6 difficulties, so use them wisely and sort them.
pub fn spread_difficulties(conf: &SimfileFix, simfiles: &mut Vec<Box<Simfile>>) -> Result<()> {
    ensure!(
        conf.out_difficulties.len() == conf.equivalent_meters.len(),
        "equivalent_meters must have the same length as out_difficulties"
    );
    if conf.out_difficulties.is_empty() {
        simfiles.clear();
        return Ok(());
    }
    //Create an auxiliary vec holding chart indices and difficulties
    let mut order = simfiles
        .iter()
        .enumerate()
        .map(|(idx, sm)| (idx, sm.difficulty()))
        .collect::<Vec<_>>();
    trace!("    raw difficulties: {:?}", order);

    //Sort by difficulty
    order.sort_by_key(|(_, d)| SortableFloat(*d));
    trace!("    sorted difficulties: {:?}", order);

    //Remove difficulties, mantaining as much spread as possible
    while order.len() > conf.out_difficulties.len() {
        //Find the smallest gap
        let (mut smallest, _) = order
            .windows(2)
            .enumerate()
            .min_by_key(|(_idx, window)| SortableFloat(window[1].1 - window[0].1))
            .unwrap();
        let get_gap_before = |idx: usize| {
            if idx <= 0 || idx >= order.len() {
                99999.
            } else {
                order[idx].1 - order[idx - 1].1
            }
        };
        if get_gap_before(smallest) > get_gap_before(smallest + 2) {
            smallest += 1;
        }
        //Remove this chart :(
        order.remove(smallest);
    }
    trace!("    with conflicts resolved: {:?}", order);

    //Reorder charts
    for chart in simfiles.iter_mut() {
        chart.difficulty_num = 0. / 0.;
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

    //Reassign difficulty names from numbers
    let mut difficulties = simfiles
        .iter()
        .map(|chart| {
            let (diff_idx, _diffnum) = conf
                .equivalent_meters
                .iter()
                .enumerate()
                .min_by_key(|(_i, &diffnum)| SortableFloat((diffnum - chart.difficulty_num).abs()))
                .unwrap();
            diff_idx as isize
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
                        && occupied_if < conf.out_difficulties.len() as isize
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
                    if occupied_if < 0 || occupied_if >= conf.out_difficulties.len() as isize {
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
                    set_to = set_to.min(conf.out_difficulties.len() as isize - 1).max(0);
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
        chart.difficulty = conf.out_difficulties[diff_idx as usize];
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