use crate::prelude::*;

pub(crate) fn convert(
    _ctx: &Ctx,
    _bmset_path: &Path,
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
    struct MeasureObj {
        beat_in_measure_scaled: i32,
        key: i32,
    }
    struct ConvCtx<'a> {
        next_idx: usize,
        cur_tp: TimingPoint,
        cur_beat_scaled: i64,
        cur_measure_beat_scaled: i64,
        timing_points: &'a [TimingPoint],
        out_bpms: Vec<(f64, f64)>,
        measure_objs: Vec<MeasureObj>,
        key_count: usize,
        measure_counter: usize,
        out_measures: String,
    }
    let mut ctx = ConvCtx {
        next_idx: 0,
        cur_tp: first_tp.clone(),
        cur_beat_scaled: 0,
        cur_measure_beat_scaled: 0,
        timing_points: &bm.timing_points[..],
        out_bpms: Vec::new(),
        measure_objs: Vec::new(),
        key_count: key_count as usize,
        measure_counter: 0,
        out_measures: String::new(),
    };
    const SNAP_FACTOR: f64 = 48.;
    const BEATS_IN_MEASURE: i32 = 4;
    /// Convert a beat length in milliseconds to beats-per-minute.
    fn beatlen_to_bpm(beat_len_ms: f64) -> f64 {
        60000. / beat_len_ms
    }
    /// Convert from a point in time to a snapped beat number, taking into account changing BPM.
    /// Should never be called with a time smaller than the last call!
    fn get_beat(ctx: &mut ConvCtx, time: f64) -> i64 {
        //Advance timing points
        while ctx.next_idx < ctx.timing_points.len() {
            let next_tp = &ctx.timing_points[ctx.next_idx];
            if next_tp.beat_len <= 0. {
                //Skip inherited timing points
            } else if time >= next_tp.time {
                //Advance to this timing point
                let adv_beat_nonscaled = (next_tp.time - ctx.cur_tp.time) / ctx.cur_tp.beat_len;
                ctx.cur_beat_scaled =
                    ctx.cur_beat_scaled + (SNAP_FACTOR * adv_beat_nonscaled).round() as i64;
                ctx.cur_tp = next_tp.clone();
                ctx.out_bpms.push((
                    ctx.cur_beat_scaled as f64 / SNAP_FACTOR,
                    beatlen_to_bpm(ctx.cur_tp.beat_len),
                ));
            } else {
                //Still within the current timing point
                break;
            }
            ctx.next_idx += 1;
        }
        //Use the current timing point to determine note beat
        ctx.cur_beat_scaled
            + ((time - ctx.cur_tp.time) / ctx.cur_tp.beat_len * SNAP_FACTOR).round() as i64
    }
    /// Flush a single measure to SM output using whatever's in `measure_objs`.
    fn flush_measure(ctx: &mut ConvCtx) -> Result<()> {
        //Extract largest simplified denominator, in prime-factorized form.
        //To obtain the actual number from prime-factorized form, use 2^pf[0] * 3^pf[1]
        fn get_denom(mut num: i32) -> [u32; 2] {
            let mut den = SNAP_FACTOR as i32;
            let mut simplify_by = [0; 2];
            for (idx, &factor) in [2, 3].iter().enumerate() {
                while num % factor == 0 && den % factor == 0 {
                    num /= factor;
                    den /= factor;
                    simplify_by[idx] += 1;
                }
            }
            simplify_by
        }
        let simplify_by = if ctx.measure_objs.is_empty() {
            SNAP_FACTOR as i32
        } else {
            let mut max_simplify_by = [u32::MAX; 2];
            for obj in ctx.measure_objs.iter() {
                let simplify_by = get_denom(obj.beat_in_measure_scaled);
                for (max_exp, exp) in max_simplify_by.iter_mut().zip(simplify_by.iter()) {
                    *max_exp = u32::min(*max_exp, *exp);
                }
            }
            2i32.pow(max_simplify_by[0]) * 3i32.pow(max_simplify_by[1])
        };
        let rows_per_beat = SNAP_FACTOR as i32 / simplify_by;
        println!(
            "using objects {:?} yielded {} (simplify by {})",
            ctx.measure_objs
                .iter()
                .map(|obj| obj.beat_in_measure_scaled)
                .collect::<Vec<_>>(),
            rows_per_beat,
            simplify_by
        );
        //Output 4x this amount of rows (if 4 beats in measure)
        let mut out_measure =
            vec![b'0'; (BEATS_IN_MEASURE * rows_per_beat) as usize * ctx.key_count];
        for obj in ctx.measure_objs.drain(..) {
            let idx = (obj.beat_in_measure_scaled / simplify_by) as usize;
            ensure!(
                obj.beat_in_measure_scaled % simplify_by == 0,
                "incorrect simplify_by ({} % {} == {} != 0)",
                obj.beat_in_measure_scaled,
                simplify_by,
                obj.beat_in_measure_scaled % simplify_by
            );
            ensure!(
                idx < (BEATS_IN_MEASURE * rows_per_beat) as usize,
                "called `flush_measure` with more than one measure in buffer (beat_in_measure_scaled = {} out of max {})",
                obj.beat_in_measure_scaled,
                BEATS_IN_MEASURE * rows_per_beat,
            );
            out_measure[idx * ctx.key_count + obj.key as usize] = b'1';
        }
        //Convert map into a string
        if ctx.measure_counter == 0 {
            //First line
            ctx.out_measures.push_str("  ");
        } else {
            //Add separator
            ctx.out_measures.push_str("\n, ");
        }
        write!(ctx.out_measures, "// Measure {}", ctx.measure_counter).unwrap();
        for row in 0..(BEATS_IN_MEASURE * rows_per_beat) as usize {
            ctx.out_measures.push('\n');
            for key in 0..ctx.key_count {
                ctx.out_measures
                    .push(out_measure[row * ctx.key_count + key as usize] as char);
            }
        }
        ctx.measure_counter += 1;
        ctx.cur_measure_beat_scaled += (BEATS_IN_MEASURE * SNAP_FACTOR as i32) as i64;
        Ok(())
    }
    /// Add a single measure object, flushing any amount of necessary measures if the measure is
    /// full.
    fn add_measure_obj(ctx: &mut ConvCtx, beat: i64, key: i32) -> Result<()> {
        //Finish any pending measures
        while (beat - ctx.cur_measure_beat_scaled) >= SNAP_FACTOR as i64 * BEATS_IN_MEASURE as i64 {
            flush_measure(ctx)?;
        }
        //Add this object
        /*println!(
            "      adding obj at beat {} (cur_measure = {})",
            beat, ctx.cur_measure_beat_scaled
        );*/
        ctx.measure_objs.push(MeasureObj {
            beat_in_measure_scaled: (beat - ctx.cur_measure_beat_scaled) as i32,
            key,
        });
        Ok(())
    }
    // Adjust for hit objects that occur before the first timing point by adding another timing
    // point even earlier.
    if let Some(first_hit) = bm.hit_objects.first() {
        while first_hit.time < first_tp.time {
            print!(
                "    first hit is at {} and first timing point at {}, moving back to ",
                first_hit.time, first_tp.time
            );
            first_tp.time -= first_tp.beat_len * BEATS_IN_MEASURE as f64;
            println!("{}", first_tp.time);
        }
        ctx.cur_tp = first_tp.clone();
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
            obj_key.is_finite() && obj_key >= 0. && obj_key < key_count,
            "invalid object x {} corresponding to key {}",
            obj.x,
            obj_key
        );
        add_measure_obj(&mut ctx, obj_beat, obj_key as i32)?;
    }
    // Flush any remaining hit objects.
    flush_measure(&mut ctx)?;
    // Create final SM file.
    let sm = Simfile {
        title: bm.title_unicode,
        title_trans: bm.title,
        subtitle: bm.version.clone(),
        subtitle_trans: bm.version,
        artist: bm.artist_unicode,
        artist_trans: bm.artist,
        genre: bm.tags,
        credit: bm.source,
        banner: None,
        background: Some(bm.background.into()),
        lyrics: None,
        cdtitle: None,
        music: Some(bm.audio.into()),
        offset: first_tp.time / 1000.,
        bpms: ctx.out_bpms,
        stops: vec![],
        sample_start: Some(bm.preview_start),
        sample_len: None,
        charts: vec![Chart {
            gamemode: "dance-single".to_string(),
            desc: "".to_string(),
            diff_name: "".to_string(),
            diff_num: 1.,
            radar: [0., 0., 0., 0., 0.],
            measures: ctx.out_measures,
        }],
    };
    Ok(sm)
}
