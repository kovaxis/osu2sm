use crate::prelude::*;

pub(crate) fn convert(
    _ctx: &Ctx,
    bmset_path: &Path,
    _bm_path: &Path,
    bm: Beatmap,
) -> Result<Simfile> {
    ensure!(
        bm.mode == osufile::MODE_MANIA,
        "mode {} not supported (only mania is currently supported)",
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
        timing_points: &'a [TimingPoint],
        out_bpms: Vec<(f64, f64)>,
        out_notes: Vec<Note>,
    }
    let mut ctx = ConvCtx {
        next_idx: 1,
        cur_tp: first_tp.clone(),
        cur_beat: BeatPos::from_float(0.),
        timing_points: &bm.timing_points[..],
        out_bpms: Vec::new(),
        out_notes: Vec::new(),
    };
    /// Convert a beat length in milliseconds to beats-per-minute.
    fn beatlen_to_bpm(beat_len_ms: f64) -> f64 {
        60000. / beat_len_ms
    }
    /// Convert from a point in time to a snapped beat number, taking into account changing BPM.
    /// Should never be called with a time smaller than the last call!
    fn get_beat(ctx: &mut ConvCtx, time: f64) -> BeatPos {
        //Advance timing points
        while ctx.next_idx < ctx.timing_points.len() {
            let next_tp = &ctx.timing_points[ctx.next_idx];
            if next_tp.beat_len <= 0. {
                //Skip inherited timing points
            } else if time >= next_tp.time {
                //Advance to this timing point
                let adv_beat_nonscaled = (next_tp.time - ctx.cur_tp.time) / ctx.cur_tp.beat_len;
                ctx.cur_beat = ctx.cur_beat + BeatPos::from_float(adv_beat_nonscaled);
                ctx.cur_tp = next_tp.clone();
                ctx.out_bpms
                    .push((ctx.cur_beat.as_float(), beatlen_to_bpm(ctx.cur_tp.beat_len)));
            } else {
                //Still within the current timing point
                break;
            }
            ctx.next_idx += 1;
        }
        //Use the current timing point to determine note beat
        ctx.cur_beat + BeatPos::from_float((time - ctx.cur_tp.time) / ctx.cur_tp.beat_len)
    }
    // Adjust for hit objects that occur before the first timing point by adding another timing
    // point even earlier.
    if let Some(first_hit) = bm.hit_objects.first() {
        while first_hit.time < first_tp.time {
            first_tp.time -= first_tp.beat_len * first_tp.meter as f64;
        }
        ctx.cur_tp = first_tp.clone();
        ctx.out_bpms.push((0., beatlen_to_bpm(first_tp.beat_len)));
    }
    // Add hit objects as measure objects, pushing out SM notedata on the fly.
    let mut last_time = -1. / 0.;
    for obj in bm.hit_objects.iter() {
        ensure!(
            obj.time >= last_time,
            "hit object occurs before previous object"
        );
        last_time = obj.time;
        let obj_beat = get_beat(&mut ctx, obj.time);
        let obj_key = (obj.x * key_count / 512.).floor();
        ensure!(
            obj_key.is_finite() && obj_key as i32 >= 0 && (obj_key as i32) < key_count as i32,
            "invalid object x {} corresponding to key {}",
            obj.x,
            obj_key
        );
        ctx.out_notes.push(Note {
            beat: obj_beat,
            key: obj_key as i32,
        });
    }
    // Generate sample length from audio file
    let default_len = 60.;
    let sample_len = if bm.audio.is_empty() {
        default_len
    } else {
        let audio_path = bmset_path.join(&bm.audio);
        match get_audio_len(&audio_path) {
            Ok(len) => (len - bm.preview_start / 1000.),
            Err(err) => {
                eprintln!(
                    "    warning: failed to get audio length for \"{}\": {:#}",
                    audio_path.display(),
                    err
                );
                default_len
            }
        }
    };
    // Create final SM file.
    let sm = Simfile {
        title: bm.title_unicode,
        title_trans: bm.title,
        subtitle: bm.version.clone(),
        subtitle_trans: bm.version.clone(),
        artist: bm.artist_unicode,
        artist_trans: bm.artist,
        genre: bm.tags,
        credit: bm.source,
        banner: None,
        background: Some(bm.background.into()),
        lyrics: None,
        cdtitle: None,
        music: Some(bm.audio.into()),
        offset: first_tp.time / -1000.,
        bpms: ctx.out_bpms,
        stops: vec![],
        sample_start: Some(bm.preview_start / 1000.),
        sample_len: Some(sample_len),
        charts: vec![Chart {
            gamemode: Gamemode::DanceSingle,
            desc: bm.version,
            difficulty: Difficulty::Edit,
            difficulty_num: 0.,
            radar: [0., 0., 0., 0., 0.],
            notes: ctx.out_notes,
        }],
    };
    Ok(sm)
}

/// Get the length of an audio file in seconds.
fn get_audio_len(path: &Path) -> Result<f64> {
    Ok(mp3_duration::from_path(path)?.as_secs_f64())
}
