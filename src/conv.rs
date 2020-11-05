//! Convert osu! beatmaps to stepmania simfiles.

use crate::prelude::*;

pub(crate) fn convert<'a>(
    ctx: &'a Ctx,
    bmset_path: &Path,
    _bm_path: &Path,
    bm: Beatmap,
) -> Result<SimfileList> {
    ensure!(
        bm.mode == osufile::MODE_MANIA,
        "mode not supported ({}) only mania (3) is currently supported",
        bm.mode
    );
    let key_count = bm.circle_size.round();
    ensure!(
        key_count.is_finite() && key_count >= 0. && key_count < 128.,
        "invalid keycount {}",
        key_count
    );
    let mut first_tp = bm
        .timing_points
        .first()
        .ok_or(anyhow!("no timing points"))?
        .clone();
    ensure!(
        first_tp.beat_len > 0.,
        "beatLength of first timing point must be positive (is {})",
        first_tp.beat_len
    );
    struct ConvCtx<'a> {
        next_idx: usize,
        cur_tp: TimingPoint,
        cur_beat: BeatPos,
        cur_beat_nonapproxed: f64,
        timing_points: &'a [TimingPoint],
        out_bpms: Vec<ControlPoint>,
        out_notes: Vec<Note>,
    }
    let mut conv = ConvCtx {
        next_idx: 1,
        cur_tp: first_tp.clone(),
        cur_beat: BeatPos::from(0.),
        cur_beat_nonapproxed: 0.,
        timing_points: &bm.timing_points[..],
        out_bpms: Vec::new(),
        out_notes: Vec::new(),
    };
    /// Convert from a point in time to a snapped beat number, taking into account changing BPM.
    /// Should never be called with a time smaller than the last call!
    fn get_beat(conv: &mut ConvCtx, time: f64) -> BeatPos {
        //Advance timing points
        while conv.next_idx < conv.timing_points.len() {
            let next_tp = &conv.timing_points[conv.next_idx];
            if next_tp.beat_len <= 0. {
                //Skip inherited timing points
            } else if time >= next_tp.time {
                //Advance to this timing point
                conv.cur_beat_nonapproxed +=
                    (next_tp.time - conv.cur_tp.time) / conv.cur_tp.beat_len;
                //Arbitrary round-to-beat because of broken beatmaps
                conv.cur_beat = BeatPos::from(conv.cur_beat_nonapproxed).round(2);
                conv.cur_tp = next_tp.clone();
                conv.out_bpms.push(ControlPoint {
                    beat: conv.cur_beat,
                    beat_len: conv.cur_tp.beat_len / 1000.,
                });
            } else {
                //Still within the current timing point
                break;
            }
            conv.next_idx += 1;
        }
        //Use the current timing point to determine note beat
        conv.cur_beat + BeatPos::from((time - conv.cur_tp.time) / conv.cur_tp.beat_len)
    }
    // Adjust for hit objects that occur before the first timing point by adding another timing
    // point even earlier.
    if let Some(first_hit) = bm.hit_objects.first() {
        while first_hit.time < first_tp.time {
            first_tp.time -= first_tp.beat_len * first_tp.meter as f64;
        }
        conv.cur_tp = first_tp.clone();
        conv.out_bpms.push(ControlPoint {
            beat: BeatPos::from(0.),
            beat_len: first_tp.beat_len / 1000.,
        });
    }
    // Add hit objects as measure objects, pushing out SM notedata on the fly.
    let mut pending_tails = Vec::new();
    let mut last_time = -1. / 0.;
    for obj in bm.hit_objects.iter() {
        //Ensure objects increase monotonically in time
        ensure!(
            obj.time >= last_time,
            "hit object occurs before previous object"
        );
        last_time = obj.time;
        //Insert any pending long note tails
        pending_tails.retain(|&(time, key)| {
            if time <= obj.time {
                // Insert now
                let end_beat = get_beat(&mut conv, time);
                conv.out_notes.push(Note {
                    kind: Note::KIND_TAIL,
                    beat: end_beat,
                    key,
                });
                false
            } else {
                // Keep waiting
                true
            }
        });
        //Get data for this object
        let obj_beat = get_beat(&mut conv, obj.time);
        let obj_key = (obj.x * key_count / 512.).floor();
        ensure!(
            obj_key.is_finite() && obj_key as i32 >= 0 && (obj_key as i32) < key_count as i32,
            "invalid object x {} corresponding to key {}",
            obj.x,
            obj_key
        );
        let obj_key = obj_key as i32;
        //Act depending on object type
        if obj.ty & osufile::TYPE_HOLD != 0 {
            // Long note
            // Get the end time in millis
            let end_time = obj
                .extras
                .split(':')
                .next()
                .unwrap_or_default()
                .parse::<f64>()
                .map_err(|_| {
                    anyhow!(
                        "invalid hold note extras \"{}\", expected endTime",
                        obj.extras
                    )
                })?;
            // Leave it for later insertion at the correct time
            let insert_idx = pending_tails
                .iter()
                .position(|(t, _)| *t > end_time)
                .unwrap_or(pending_tails.len());
            pending_tails.insert(insert_idx, (end_time, obj_key));
            // Insert the long note head
            conv.out_notes.push(Note {
                kind: Note::KIND_HEAD,
                beat: obj_beat,
                key: obj_key,
            });
        } else if obj.ty & osufile::TYPE_HIT != 0 {
            // Hit note
            conv.out_notes.push(Note {
                kind: Note::KIND_HIT,
                beat: obj_beat,
                key: obj_key,
            });
        }
    }
    // Push out any pending long note tails
    for (time, key) in pending_tails {
        let end_beat = get_beat(&mut conv, time);
        conv.out_notes.push(Note {
            kind: '3',
            beat: end_beat,
            key,
        });
    }
    // Generate sample length from audio file
    let default_len = 60.;
    let sample_len = if bm.audio.is_empty() || !ctx.opts.query_audio_len {
        default_len
    } else {
        let audio_path = bmset_path.join(&bm.audio);
        let (len, result) = get_audio_len(&audio_path);
        if let Err(err) = result {
            warn!(
                "    warning: failed to get full audio length for \"{}\": {:#}",
                audio_path.display(),
                err
            );
        }
        (len - bm.preview_start / 1000.).max(10.)
    };
    // Create the final SM file in all supported gamemodes
    let gamemodes = ctx
        .opts
        .gamemodes
        .iter()
        .copied()
        .filter(|gm| gm.key_count() == key_count as i32);
    let mut out = Vec::with_capacity(gamemodes.clone().count());
    for gamemode in gamemodes {
        out.push(Box::new(Simfile {
            title: if ctx.opts.unicode {
                bm.title_unicode.clone()
            } else {
                bm.title.clone()
            },
            title_trans: bm.title.clone(),
            subtitle: bm.version.clone(),
            subtitle_trans: bm.version.clone(),
            artist: if ctx.opts.unicode {
                bm.artist_unicode.clone()
            } else {
                bm.artist.clone()
            },
            artist_trans: bm.artist.clone(),
            genre: String::new(),
            credit: bm.creator.clone(),
            banner: None,
            background: Some(bm.background.clone().into()),
            lyrics: None,
            cdtitle: None,
            music: Some(bm.audio.clone().into()),
            offset: first_tp.time / -1000.,
            bpms: conv.out_bpms.clone(),
            stops: vec![],
            sample_start: Some(bm.preview_start / 1000.),
            sample_len: Some(sample_len),
            gamemode,
            desc: bm.version.clone(),
            difficulty: Difficulty::Edit,
            difficulty_num: 0.,
            radar: [0., 0., 0., 0., 0.],
            notes: conv.out_notes.clone(),
        }));
    }
    Ok(out)
}

/// Get the length of an audio file in seconds.
fn get_audio_len(path: &Path) -> (f64, Result<()>) {
    let (len, result) = match mp3_duration::from_path(path) {
        Ok(len) => (len, Ok(())),
        Err(err) => (err.at_duration, Err(err.into())),
    };
    (len.as_secs_f64(), result)
}
