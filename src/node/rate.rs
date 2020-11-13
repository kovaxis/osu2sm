//! Rate a simfile's difficulty.
//!
//! Perhaps one day the groove meter and whatnot could be updated, but for now it's just
//! in-practice BPM estimation.

use crate::node::prelude::*;

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
            method: RateMethod::Density(default()),
            scale: [0., 1., 0., 60.],
            set_meter: true,
            set_diff: vec![
                (60., Beginner),
                (100., Easy),
                (140., Medium),
                (180., Hard),
                (220., Challenge),
                (260., Edit),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RateMethod {
    /// Use the raw total amount of non-tail notes.
    Count(NoteCount),
    /// Use a weighted norm of note densities, where each note may have several rectangular "halos".
    ///
    /// Outputs the density in note units / sec.
    /// Scale `x60` to obtain effective BPM.
    Density(NoteDensity),
    /// Use the norm of the gaps between notes.
    ///
    /// Outputs the "average" note density in notes / sec.
    /// Scale `x60` to obtain effective BPM.
    Gap(NoteGap),
}
impl Default for RateMethod {
    fn default() -> Self {
        Self::Density(default())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteCount {
    /// Whether to take the logarithm of the amount of notes, instead of the raw amount
    /// itself.
    ///
    /// More specifically, if `log` is greater than zero, take the logarithm base `log` of the
    /// amount of non-tail notes.
    pub log: f64,
}
impl Default for NoteCount {
    fn default() -> Self {
        Self { log: 0. }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteDensity {
    /// A list of `(duration, weight)` pairs.
    pub halos: Vec<(f64, f64)>,
    /// A list of weights for each additional simultaneous note.
    ///
    /// If more than the length of this `Vec` simultaneous notes occur, the last weight (or `1` if
    /// there are no weights) will be used.
    pub simultaneous: Vec<f64>,
    /// How much weight to give to short high densities over long low densities.
    pub exponent: f64,
}
impl Default for NoteDensity {
    fn default() -> Self {
        Self {
            halos: vec![(2., 0.), (1., 1.)],
            simultaneous: vec![1., 0.75, 0.5],
            exponent: 2.,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteGap {
    /// How much weight to give to few very short gaps over many not-so-short gaps.
    pub exponent: f64,
}
impl Default for NoteGap {
    fn default() -> Self {
        Self { exponent: 2. }
    }
}

impl Node for Rate {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                rate(self, sm)?;
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

fn rate(conf: &Rate, sm: &mut Simfile) -> Result<()> {
    let computed = match &conf.method {
        RateMethod::Count(conf) => get_note_count(conf, sm),
        RateMethod::Density(conf) => get_note_density(conf, sm),
        RateMethod::Gap(conf) => get_note_gap(conf, sm),
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

fn get_note_count(conf: &NoteCount, sm: &Simfile) -> f64 {
    let mut count = 0;
    for note in sm.notes.iter() {
        if !note.is_tail() {
            count += 1;
        }
    }
    if conf.log > 0. {
        (count as f64).log(conf.log)
    } else {
        count as f64
    }
}

fn get_note_density(conf: &NoteDensity, sm: &Simfile) -> f64 {
    let mut to_time = sm.beat_to_time();
    let halo_densities = conf
        .halos
        .iter()
        .map(|&(duration, weight)| (duration / 2., (weight / duration) as f32))
        .collect::<Vec<_>>();
    let mut default_base_weight = 0.;
    let mut default_key_weight = 1.;
    let mut key_weights = Vec::with_capacity(conf.simultaneous.len());
    {
        let mut acc = 0.;
        for &w in conf.simultaneous.iter() {
            acc += w;
            key_weights.push(acc as f32);
            default_base_weight = acc as f32;
            default_key_weight = w as f32;
        }
    }
    let mut last_id: u32 = 0;
    let mut weight_changes = Vec::with_capacity(2 * sm.notes.len() * conf.halos.len());
    for beat in sm.iter_beats() {
        let time = to_time.beat_to_time(beat.pos);
        //Calculate a weight for the notes on this beat
        let note_count = beat.count_heads(&sm.notes);
        if note_count > 0 {
            let weight = key_weights.get(note_count - 1).copied().unwrap_or_else(|| {
                default_base_weight + default_key_weight * (note_count - key_weights.len()) as f32
            });
            //Create halos for this note weight
            for &(radius, density) in halo_densities.iter() {
                last_id += 1;
                weight_changes.push((time - radius, last_id, weight * density));
                weight_changes.push((time + radius, last_id, f32::NAN));
            }
        }
    }
    weight_changes.sort_unstable_by_key(|(time, _id, _change)| SortableFloat(*time));
    if weight_changes.is_empty() {
        return 0.;
    }
    let mut total_density = 0.;
    let mut cur_time = weight_changes[0].0;
    // OPTIMIZE: Use fixed-point for density, keeping track of `cur_density` without keeping track
    // of individual halos. Fixed-point would allow for the needed precision.
    let mut active_halos = Vec::new();
    let mut total_time: f64 = 0.;
    for (time, id, change) in weight_changes {
        //Sum density
        let mut cur_density: f32 = 0.;
        for &(_halo_id, halo_density) in active_halos.iter() {
            cur_density += halo_density;
        }
        let dt = time - cur_time;
        total_density += dt as f32 * cur_density.powf(conf.exponent as f32);
        if !active_halos.is_empty() {
            total_time += dt;
        }
        //Update for next iteration
        cur_time = time;
        if change.is_nan() {
            for i in 0..active_halos.len() {
                if active_halos[i].0 == id {
                    active_halos.remove(i);
                    break;
                }
            }
        } else {
            active_halos.push((id, change));
        }
    }
    if total_time > 0. {
        (total_density as f64 / total_time).powf(1. / conf.exponent)
    } else {
        0.
    }
}

fn get_note_gap(conf: &NoteGap, sm: &Simfile) -> f64 {
    let exp = conf.exponent as f32;
    let mut last_time = None;
    let mut to_time = sm.beat_to_time();
    let mut total_freq = 0.;
    let mut total_gaps = 0;
    for beat in sm.iter_beats() {
        if beat.count_heads(&sm.notes) == 0 {
            continue;
        }
        let time = to_time.beat_to_time(beat.pos);
        if let Some(last_time) = last_time {
            let gap = (time - last_time) as f32;
            if gap > 0. {
                let freq = 1. / gap;
                total_freq += freq.powf(exp);
                total_gaps += 1;
            }
        }
        last_time = Some(time);
    }
    if total_gaps <= 0 {
        total_freq = 0.;
    } else {
        total_freq = (total_freq / total_gaps as f32).powf(1. / exp);
    }
    total_freq as f64
}
