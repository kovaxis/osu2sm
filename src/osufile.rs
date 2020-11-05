//! Parse and handle osu! beatmaps.

use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct Beatmap {
    pub audio: String,
    pub preview_start: f64,
    pub mode: i32,
    pub mania_special: bool,
    pub title_unicode: String,
    pub title: String,
    pub artist_unicode: String,
    pub artist: String,
    pub creator: String,
    pub version: String,
    pub source: String,
    pub tags: String,
    pub id: i64,
    pub set_id: i64,
    pub hp_drain: f64,
    pub circle_size: f64,
    pub overall_difficulty: f64,
    pub approach_rate: f64,
    pub slider_multiplier: f64,
    pub slider_tickrate: f64,
    pub background: String,
    pub video: String,
    pub timing_points: Vec<TimingPoint>,
    pub hit_objects: Vec<HitObject>,
}
impl Default for Beatmap {
    fn default() -> Self {
        Beatmap {
            audio: default(),
            preview_start: 0.,
            mode: 0,
            mania_special: false,
            title_unicode: default(),
            title: default(),
            artist_unicode: default(),
            artist: default(),
            creator: default(),
            version: default(),
            source: default(),
            tags: default(),
            id: -1,
            set_id: -1,
            hp_drain: 0.,
            circle_size: 0.,
            overall_difficulty: 0.,
            approach_rate: 0.,
            slider_multiplier: 1.,
            slider_tickrate: 1.,
            background: default(),
            video: default(),
            timing_points: default(),
            hit_objects: default(),
        }
    }
}
impl Beatmap {
    pub fn parse(offset_ms: f64, path: &Path) -> Result<Beatmap> {
        use Category::*;

        #[derive(Copy, Clone, Debug)]
        enum Category {
            General,
            Metadata,
            Difficulty,
            Events,
            TimingPoints,
            HitObjects,
            Unknown,
        }

        fn strip_line(line: &str) -> &str {
            line.find("//").map(|c| &line[..c]).unwrap_or(line).trim()
        }

        fn parse_as<T: std::str::FromStr>(s: &str, name: &str) -> Result<T> {
            s.parse::<T>()
                .map_err(|_| anyhow!("invalid {} \"{}\"", name, s))
        }

        fn parse_filename(path: &str) -> String {
            if path.starts_with('"') && path.ends_with('"') {
                &path[1..path.len() - 1]
            } else {
                path
            }
            .to_string()
        }

        fn get_component<'a, T: std::str::FromStr, I: Iterator<Item = &'a str>>(
            iter: &mut I,
            name: &str,
        ) -> Result<T> {
            let textual = iter
                .next()
                .ok_or_else(|| anyhow!("expected {}, found end-of-line", name))?
                .trim();
            textual
                .parse::<T>()
                .map_err(|_| anyhow!("invalid {} \"{}\"", name, textual))
        }

        let mut category = Category::Unknown;
        let mut bm = Beatmap::default();
        let mut lines = BufReader::new(File::open(path).context("open file")?).lines();
        let mut line_num = 0;

        //Find osu header
        let mut global_offset = offset_ms;
        for line in &mut lines {
            let line = line?;
            line_num += 1;
            //Remove stupid UTF-8 BOM
            let line = strip_line(line.trim_start_matches('\u{feff}'));
            if !line.is_empty() {
                let prefix = "osu file format v";
                ensure!(line.starts_with(prefix), "not an osu! beatmap file");
                let version = parse_as::<i32>(&line[prefix.len()..], "osu format version")?;
                // According to the osu!lazer source:
                // BeatmapVersion 4 and lower had an incorrect offset (stable has this set as 24ms off)
                if version < 5 {
                    global_offset += 24.;
                }
                break;
            }
        }
        let global_offset = global_offset;

        let mut errors = Vec::new();
        let mut requires_sort = false;
        let mut last_time = -1. / 0.;
        for line in lines {
            let line = line?;
            line_num += 1;
            let line = strip_line(&line);
            let result = (|| -> Result<()> {
                let split = |sep: &str| {
                    line.find(sep)
                        .map(|idx| (&line[..idx], line[idx + sep.len()..].trim()))
                };
                if line.is_empty() {
                } else if line.starts_with('[') && line.ends_with(']') {
                    category = match &line[1..line.len() - 1] {
                        "General" => General,
                        "Metadata" => Metadata,
                        "Difficulty" => Difficulty,
                        "Events" => Events,
                        "TimingPoints" => TimingPoints,
                        "HitObjects" => HitObjects,
                        _ => Unknown,
                    };
                } else {
                    match category {
                        General => {
                            if let Some((k, v)) = split(":") {
                                match k {
                                    "AudioFilename" => bm.audio = parse_filename(v),
                                    "PreviewTime" => {
                                        bm.preview_start = parse_as::<f64>(v, "PreviewTime")?
                                    }
                                    "Mode" => bm.mode = parse_as::<i32>(v, "Mode")?,
                                    "SpecialStyle" => {
                                        bm.mania_special = parse_as::<i32>(v, "ManiaSpecial")? != 0
                                    }

                                    _ => {}
                                }
                            }
                        }
                        Metadata => {
                            if let Some((k, v)) = split(":") {
                                match k {
                                    "Title" => bm.title = v.to_string(),
                                    "TitleUnicode" => bm.title_unicode = v.to_string(),
                                    "Artist" => bm.artist = v.to_string(),
                                    "ArtistUnicode" => bm.artist_unicode = v.to_string(),
                                    "Creator" => bm.creator = v.to_string(),
                                    "Version" => bm.version = v.to_string(),
                                    "Source" => bm.source = v.to_string(),
                                    "Tags" => bm.tags = v.to_string(),
                                    "BeatmapID" => bm.id = parse_as::<i64>(v, "BeatmapID")?,
                                    "BeatmapSetID" => {
                                        bm.set_id = parse_as::<i64>(v, "BeatmapSetID")?
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Difficulty => {
                            if let Some((k, v)) = split(":") {
                                let v = parse_as::<f64>(v, k);
                                match k {
                                    "HPDrainRate" => bm.hp_drain = v?,
                                    "CircleSize" => bm.circle_size = v?,
                                    "OverallDifficulty" => bm.overall_difficulty = v?,
                                    "ApproachRate" => bm.approach_rate = v?,
                                    "SliderMultiplier" => bm.slider_multiplier = v?,
                                    "SliderTickRate" => bm.slider_tickrate = v?,
                                    _ => {}
                                }
                            }
                        }
                        Events => {
                            let mut comps = line.split(',');
                            match &get_component::<String, _>(&mut comps, "event type")?[..] {
                                ty @ "0" | ty @ "1" | ty @ "Video" => {
                                    let _start_time: String =
                                        get_component(&mut comps, "start time")?;
                                    let filename: String = get_component(&mut comps, "filename")?;
                                    let filename = parse_filename(&filename);
                                    if ty == "0" {
                                        bm.background = filename;
                                    } else {
                                        bm.video = filename;
                                    }
                                }
                                _ => {}
                            }
                        }
                        TimingPoints => {
                            let mut comps = line.split(',');
                            let time = get_component::<f64, _>(&mut comps, "time")? + global_offset;
                            let beat_len = get_component(&mut comps, "beatLength")?;
                            let meter = comps
                                .next()
                                .unwrap_or_default()
                                .trim()
                                .parse::<i32>()
                                .unwrap_or(4);
                            bm.timing_points.push(TimingPoint {
                                time,
                                beat_len,
                                meter,
                            });
                        }
                        HitObjects => {
                            let mut comps = line.splitn(6, ',');
                            let x = get_component(&mut comps, "x")?;
                            let y = get_component(&mut comps, "y")?;
                            let time = get_component::<f64, _>(&mut comps, "time")? + global_offset;
                            let ty = get_component(&mut comps, "type")?;
                            let _hitsound: String = get_component(&mut comps, "hitsound")?;
                            let extras = comps.next().unwrap_or_default().trim().to_string();
                            bm.hit_objects.push(HitObject {
                                x,
                                y,
                                time,
                                ty,
                                extras,
                            });
                            if time < last_time {
                                requires_sort = true;
                            }
                            last_time = time;
                        }
                        Unknown => {}
                    }
                }
                Ok(())
            })();
            if let Err(err) = result {
                errors.push((line_num, line.to_string(), err));
            }
        }
        if !errors.is_empty() {
            warn!("  warnings parsing \"{}\":", path.display());
            for (line_num, line, err) in errors.iter() {
                warn!("    line {} (\"{}\"): {:#}", line_num, line, err);
            }
        }
        //Turns out hitobjects _can_ be out-of-order, according to the lazer source and actual
        //ranked beatmaps
        if requires_sort {
            bm.hit_objects.sort_by_key(|obj| SortableFloat(obj.time));
        }
        Ok(bm)
    }
}

#[derive(Debug, Clone)]
pub struct TimingPoint {
    pub time: f64,
    pub beat_len: f64,
    pub meter: i32,
}

#[derive(Debug, Clone)]
pub struct HitObject {
    pub x: f64,
    pub y: f64,
    pub time: f64,
    pub ty: u32,
    pub extras: String,
}

pub const MODE_STD: i32 = 0;
pub const MODE_TAIKO: i32 = 1;
pub const MODE_CATCH: i32 = 2;
pub const MODE_MANIA: i32 = 3;

pub const TYPE_HIT: u32 = 1 << 0;
pub const TYPE_SLIDER: u32 = 1 << 1;
pub const TYPE_SPINNER: u32 = 1 << 3;
pub const TYPE_HOLD: u32 = 1 << 7;
