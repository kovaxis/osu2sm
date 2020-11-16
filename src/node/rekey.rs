use crate::node::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Rekey {
    pub from: BucketId,
    pub into: BucketId,
    /// Into what gamemode to convert.
    pub gamemode: Gamemode,
    /// If the input keycount is the same as the output keycount, do not remap.
    pub avoid_shuffle: bool,
    /// Weighting options to prevent too many jacks (quick notes on the same key).
    ///
    /// Each element consists of a `(time, weight)` pair, where `time` is the time elapsed since
    /// this key was last active, and `weight` is the random choice weight to assign to this key.
    /// Intermediate values are interpolated.
    ///
    /// This way, keys that have not had notes in a while have a higher chance of getting a key,
    /// while keys that just had a key will not get spammed at random.
    pub weight_curve: Vec<(f32, f32)>,
}
impl Default for Rekey {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            gamemode: Gamemode::DanceSingle,
            avoid_shuffle: true,
            weight_curve: vec![(0., 1.), (0.4, 10.), (0.8, 200.), (1.4, 300.)],
        }
    }
}

impl Node for Rekey {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, list| {
            for sm in list.iter_mut() {
                rekey(sm, self)?;
                sm.gamemode = self.gamemode;
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

pub struct KeyAlloc {
    weight_points: Vec<(f32, f32, f32)>,
    default_weight: f32,
    last_active: Vec<f64>,
}
impl KeyAlloc {
    pub fn new(key_count: usize) -> KeyAlloc {
        KeyAlloc {
            weight_points: Vec::new(),
            default_weight: 1.,
            last_active: vec![f64::NEG_INFINITY; key_count],
        }
    }

    pub fn set_weight_curve(&mut self, weight_curve: &[(f32, f32)]) {
        self.weight_points.clear();
        self.weight_points
            .reserve(weight_curve.len().saturating_sub(1));
        for i in 0..weight_curve.len().saturating_sub(1) {
            let (this_x, this_y) = weight_curve[i];
            let (next_x, next_y) = weight_curve[i + 1];
            let m = (next_y - this_y) / (next_x - this_x);
            self.weight_points.push((next_x, m, -this_x * m + this_y));
        }
        self.default_weight = weight_curve.last().map(|(_x, y)| *y).unwrap_or(1.);
    }

    /// Map the amount of time since the key was last active to a choose weight.
    /// Try to map higher times to more weight, but not too much
    pub fn inactive_time_to_weight(&self, time: f32) -> f32 {
        for &(up_to, m, c) in self.weight_points.iter() {
            if time <= up_to {
                return m * time + c;
            }
        }
        self.default_weight
    }

    pub fn touch(&mut self, key: usize, time: f64) {
        self.last_active[key] = time;
    }

    /// The `keys` argument can be in an arbitrary order.
    pub fn alloc(&mut self, keys: &[usize], time: f64, rng: &mut FastRng) -> Option<usize> {
        match keys.choose_weighted(rng, |&out_key| {
            let time = (time - self.last_active[out_key]) as f32;
            let weight = self.inactive_time_to_weight(time);
            weight
        }) {
            Ok(&key) => {
                self.touch(key, time);
                Some(key)
            }
            Err(_) => None,
        }
    }

    pub fn alloc_idx(
        &mut self,
        keys: &[usize],
        time: f64,
        rng: &mut FastRng,
    ) -> Option<(usize, usize)> {
        self.alloc(keys, time, rng)
            .map(|key| (keys.iter().position(|&k| k == key).unwrap(), key))
    }
}

/// Shuffle keys around, changing keycount in the way.
fn rekey(sm: &mut Simfile, conf: &Rekey) -> Result<()> {
    //Keycounts
    let in_keycount = sm.gamemode.key_count() as usize;
    let out_keycount = conf.gamemode.key_count() as usize;
    ensure!(in_keycount > 0, "cannot convert 0-key map");
    ensure!(out_keycount > 0, "cannot convert to 0-key map");

    //Avoid unnecessary rekeys
    if conf.avoid_shuffle && in_keycount == out_keycount {
        trace!(
            "    avoiding {}K -> {}K conversion",
            in_keycount,
            out_keycount,
        );
        return Ok(());
    }
    trace!("    converting {}K to {}K", in_keycount, out_keycount);

    //The strategy used to choose keys
    let mut key_alloc = KeyAlloc::new(out_keycount);
    key_alloc.set_weight_curve(&conf.weight_curve);

    //Detach note buffer for lifetiming purposes
    let mut notes = mem::replace(&mut sm.notes, Vec::new());
    //To randomize key mappings
    let mut rng = simfile_rng(sm, "rekey");
    //Beat -> time
    let mut to_time = ToTime::new(sm);

    //Holds which outkeys are locked.
    //If the inner option is `Some`, that outkey should be unlocked after that beat passes.
    let mut locked_outkeys = vec![None; out_keycount];
    //If a tail occurs at the given inkey, unlock the stored outkey.
    let mut unlock_by_tails = vec![0; in_keycount];
    //Auxiliary buffer to choose weighted outkeys
    let mut choose_tmp_buf = Vec::with_capacity(out_keycount);

    for note in notes.iter_mut() {
        let note_time = to_time.beat_to_time(note.beat);
        //Unlock any auto-unlocking keys
        for locked in locked_outkeys.iter_mut() {
            if let Some(Some(unlock_after)) = *locked {
                if note.beat > unlock_after {
                    *locked = None;
                }
            }
        }
        //Map key
        let mapped_key = if note.is_tail() {
            let out_key = unlock_by_tails[note.key as usize];
            locked_outkeys[out_key] = None;
            key_alloc.touch(out_key, note_time);
            out_key as i32
        } else {
            //Choose an outkey using randomness and weights
            choose_tmp_buf.clear();
            choose_tmp_buf.extend(
                locked_outkeys
                    .iter()
                    .enumerate()
                    .filter(|(_i, locked)| locked.is_none())
                    .map(|(i, _locked)| i),
            );
            match key_alloc.alloc(&choose_tmp_buf, note_time, &mut rng) {
                Some(out_key) => {
                    if note.is_head() {
                        locked_outkeys[out_key] = Some(None);
                        unlock_by_tails[note.key as usize] = out_key;
                    } else {
                        locked_outkeys[out_key] = Some(Some(note.beat));
                    }
                    out_key as i32
                }
                None => {
                    //All output keys are locked
                    -1
                }
            }
        };
        note.key = mapped_key;
    }
    notes.retain(|note| note.key >= 0);
    sm.notes = notes;
    Ok(())
}
