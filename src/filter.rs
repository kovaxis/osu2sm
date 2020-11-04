//! Filters to apply to parsed beatmaps.

use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Filter {
    ConvertTo(ConvertTo),
    Whitelist(Vec<Gamemode>),
    Blacklist(Vec<Gamemode>),
}
impl Filter {
    pub fn apply(&self, sm: &mut Simfile) -> Result<(bool, SimfileList)> {
        Ok(match self {
            Filter::ConvertTo(conv) => (true, batch_convert_to(sm, conv)?),
            Filter::Whitelist(gms) => (should_keep(sm, gms, true), Vec::new()),
            Filter::Blacklist(gms) => (should_keep(sm, gms, false), Vec::new()),
        })
    }
}

fn should_keep(sm: &Simfile, gms: &[Gamemode], white: bool) -> bool {
    gms.iter().any(|gm| *gm == sm.gamemode) == white
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertTo {
    pub into: Vec<Gamemode>,
    pub keep_original: bool,
}

fn batch_convert_to(sm: &mut Simfile, conv: &ConvertTo) -> Result<SimfileList> {
    let mut out = Vec::with_capacity(conv.into.len());
    for (idx, &gm) in conv.into.iter().enumerate() {
        if !conv.keep_original && idx + 1 == conv.into.len() {
            convert_to(sm, gm)?;
        } else {
            let mut tmp = Box::new(sm.clone());
            convert_to(&mut tmp, gm)?;
            out.push(tmp);
        }
    }
    Ok(out)
}

fn convert_to(sm: &mut Simfile, new_gm: Gamemode) -> Result<()> {
    //Keycounts
    let in_keycount = sm.gamemode.key_count() as usize;
    let out_keycount = new_gm.key_count() as usize;
    trace!("    converting {}K to {}K", in_keycount, out_keycount);
    ensure!(in_keycount > 0, "cannot convert 0-key map");
    ensure!(out_keycount > 0, "cannot convert to 0-key map");

    //Detach note buffer for lifetiming purposes
    let mut notes = mem::replace(&mut sm.notes, Vec::new());
    //To randomize key mappings
    let mut rng = FastRng::seed_from_u64(fxhash::hash64(&(&sm.music, &sm.title_trans, &sm.desc)));

    let mut locked_outkeys = vec![None; out_keycount];
    let mut unlock_by_tails = vec![0; in_keycount];

    for note in notes.iter_mut() {
        trace!(
            "      handling note '{}' at beat {}, key {}",
            note.kind,
            note.beat.as_float(),
            note.key
        );
        //Unlock any auto-unlocking keys
        for (out_key, locked) in locked_outkeys.iter_mut().enumerate() {
            if let Some(Some(unlock_after)) = *locked {
                if note.beat > unlock_after {
                    trace!(
                        "        unlocked outkey {}, because beat {} has passed",
                        out_key,
                        unlock_after.as_float()
                    );
                    *locked = None;
                }
            }
        }
        //Map key
        let mapped_key = if note.is_tail() {
            let out_key = unlock_by_tails[note.key as usize];
            locked_outkeys[out_key] = None;
            trace!(
                "        unlocked outkey {}, because a tail was encountered",
                out_key
            );
            out_key as i32
        } else {
            match locked_outkeys
                .iter()
                .enumerate()
                .filter(|(_i, locked)| locked.is_none())
                .choose(&mut rng)
            {
                Some((out_key, _)) => {
                    if note.is_head() {
                        trace!(
                            "        locking outkey {} until a tail is encountered on inkey {}",
                            out_key,
                            note.key
                        );
                        locked_outkeys[out_key] = Some(None);
                        unlock_by_tails[note.key as usize] = out_key;
                    } else {
                        trace!(
                            "        locking outkey {} temporarily for beat {}",
                            out_key,
                            note.beat.as_float()
                        );
                        locked_outkeys[out_key] = Some(Some(note.beat));
                    }
                    out_key as i32
                }
                None => {
                    //All output keys are locked
                    trace!("        ran out of output keys ({:?})", locked_outkeys);
                    -1
                }
            }
        };
        note.key = mapped_key;
    }
    notes.retain(|note| note.key >= 0);

    /*
    let redirect_interval_min = 1.;
    let redirect_interval_max = 2.;
    let speculative_weight = 1.;
    //Maps from beats to times in an efficient manner
    let mut to_time = ToTime::new(sm);

    //Which keys are "locked" due to hold notes
    let mut active_keys = vec![false; in_keycount];

    // Mapping from source keys to output keys
    // Using a 3-key to 2-key example:
    //
    // Output (keycount = 2)
    // 0   1
    // ^   ^
    // |   |
    // (dynamic shuffle)
    // ^   ^
    // |   |
    // 0   1
    // ^   ^
    // +-------+
    // |   |   |
    // 0   1   2
    // ^   ^   ^
    // |   |   |
    // (dynamic shuffle)
    // ^   ^   ^
    // |   |   |
    // 0   1   2
    // Input (keycount = 3)
    let mut out_mapping = (0..out_keycount).collect::<Vec<_>>();
    let mut in_mapping = (0..in_keycount).collect::<Vec<_>>();
    //Keeps track of how many input keys are mapped to each output key
    let mut out_key_population = vec![0; out_keycount];
    for in_key in 0..in_keycount {
        let mapped = out_mapping[in_mapping[in_key as usize] % out_keycount];
        out_key_population[mapped] += 1;
    }

    //Schedule the next shuffle, in seconds and amount of keys to reposition
    let mut next_shuffle = (to_time.beat_to_time(BeatPos::from_float(0.)), in_keycount);
    //Integral of population over time
    //We strive to make this as even as possible
    let mut densities_last_updated = next_shuffle.0;
    let mut total_densities = vec![0.; out_keycount];

    for note in notes.iter_mut() {
        ensure!(
            note.key >= 0 && note.key < in_keycount as i32,
            "note key {} out of the range [0, {})",
            note.key,
            in_keycount
        );
        //Shuffle if it's time
        let time = to_time.beat_to_time(note.beat);
        trace!(
            "note {{ key: {}, beat: {}, time: {} }}",
            note.key,
            note.beat,
            time
        );
        if time >= next_shuffle.0 && !note.is_tail() {
            trace!("  shuffling");
            //Track densities
            {
                let dt = time - densities_last_updated;
                densities_last_updated = time;
                for (i, density) in total_densities.iter_mut().enumerate() {
                    *density += out_key_population[i] as f64 * dt;
                }
            }
            trace!("  densities: {:?}", total_densities);
            //Select shuffle candidates, priorizing unfavored extremes
            let mut to_shuffle = (0..);

            /*
            //While there are redirects pending...
            while next_redirect.1 > 0 {
                //Find a redirect source and new target
                let (old_dst_candidate, _max_density) = total_densities
                    .iter()
                    .enumerate()
                    .max_by_key(|(i, d)| {
                        if out_key_population[*i] == 0 {
                            SortableFloat(-1.)
                        } else {
                            SortableFloat(**d + out_key_population[*i] as f64 * speculative_weight)
                        }
                    })
                    .unwrap();
                trace!("  max dst density candidate: {}", old_dst_candidate);
                let iter_idx = rng.gen_range(0, out_key_population[old_dst_candidate]);
                let redirect_src = key_mapping
                    .iter()
                    .enumerate()
                    .filter(|(_src, dst)| **dst == old_dst_candidate as i32)
                    .map(|(src, _dst)| src)
                    .nth(iter_idx)
                    .unwrap();
                trace!("  dst -> src: {} -> {}", old_dst_candidate, redirect_src);
                let (redirect_dst, _min_density) = total_densities
                    .iter()
                    .enumerate()
                    .min_by_key(|(_i, d)| SortableFloat(**d))
                    .unwrap();
                trace!("  min new dst: {}", redirect_dst);
                //Carry out redirection
                trace!(
                    "  redirecting {} from -> {} to -> {}",
                    redirect_src,
                    key_mapping[redirect_src],
                    redirect_dst
                );
                out_key_population[key_mapping[redirect_src as usize] as usize] -= 1;
                key_mapping[redirect_src as usize] = redirect_dst as i32;
                out_key_population[redirect_dst as usize] += 1;
            }
            */
            trace!("  mapping looks like {:?} -> {:?}", in_mapping, out_mapping);
            next_shuffle = (
                next_shuffle.0 + rng.gen_range(redirect_interval_min, redirect_interval_max),
                rng.gen_range(2, in_keycount + 1),
            );
            trace!(
                "  scheduled next {}-key shuffle at {}s",
                next_shuffle.1,
                next_shuffle.0,
            );
        }
        //Take care with some note kinds
        if note.is_head() {
            //This key is locked to changes until a tail is found
            active_keys[note.key as usize] = true;
        } else if note.is_tail() {
            //Unlock this key
            active_keys[note.key as usize] = false;
        } else {
            //Interpret all other note types as self-contained
        }
        //Remap this note
        let mapped = out_mapping[in_mapping[note.key as usize] % out_keycount];
        trace!("  mapping key {} to {}", note.key, mapped,);
        note.key = mapped as i32;
    }*/
    //Finally, modify simfile
    sm.notes = notes;
    sm.gamemode = new_gm;
    Ok(())
}
