use crate::prelude::*;

#[derive(Debug, Clone)]
pub(crate) struct Beatmap {
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
            timing_points: default(),
            hit_objects: default(),
        }
    }
}
impl Beatmap {
    pub(crate) fn parse(path: &Path) -> Result<Beatmap> {
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
        let mut category = Category::Unknown;
        let mut bm = Beatmap::default();
        let mut lines = BufReader::new(File::open(path).context("open file")?).lines();
        ensure!(
            lines
                .next()
                .map(|res| res.unwrap_or_default())
                .unwrap_or_default()
                //Remove stupid UTF-8 BOM
                .trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with("osu file format v"),
            "not an osu! beatmap file"
        );
        fn parse_as<T: std::str::FromStr>(s: &str, name: &str) -> Result<T> {
            s.parse::<T>()
                .map_err(|_| anyhow!("invalid {} \"{}\"", name, s))
        }
        fn get_component<'a, T: std::str::FromStr + 'a, I: Iterator<Item = &'a str>>(
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
        let mut errors = Vec::new();
        let mut line_num = 1;
        for line in lines {
            let line = line?;
            let line = line.trim();
            line_num += 1;
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
                                    "AudioFilename" => bm.audio = v.to_string(),
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
                                "0" | "1" | "Video" => {
                                    let _start_time: String =
                                        get_component(&mut comps, "start time")?;
                                    let filename: String = get_component(&mut comps, "filename")?;
                                    bm.background = filename;
                                }
                                _ => {}
                            }
                        }
                        TimingPoints => {
                            let mut comps = line.split(',');
                            let time = get_component(&mut comps, "time")?;
                            let beat_len = get_component(&mut comps, "beatLength")?;
                            let meter = get_component(&mut comps, "meter")?;
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
                            let time = get_component(&mut comps, "time")?;
                            let ty = get_component(&mut comps, "type")?;
                            let _hitsound: String = get_component(&mut comps, "hitsound")?;
                            let extras = get_component(&mut comps, "extras")?;
                            bm.hit_objects.push(HitObject {
                                x,
                                y,
                                time,
                                ty,
                                extras,
                            });
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
            eprintln!("  warnings parsing \"{}\":", path.display());
            for (line_num, line, err) in errors.iter() {
                eprintln!("    line {} (\"{}\"): {:#}", line_num, line, err);
            }
        }
        Ok(bm)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TimingPoint {
    pub time: f64,
    pub beat_len: f64,
    pub meter: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct HitObject {
    pub x: f64,
    pub y: f64,
    pub time: f64,
    pub ty: u32,
    pub extras: String,
}

pub(crate) const MODE_STD: i32 = 0;
pub(crate) const MODE_TAIKO: i32 = 1;
pub(crate) const MODE_CATCH: i32 = 2;
pub(crate) const MODE_MANIA: i32 = 3;

pub(crate) const TYPE_HIT: u32 = 1 << 0;
pub(crate) const TYPE_SLIDER: u32 = 1 << 1;
pub(crate) const TYPE_SPINNER: u32 = 1 << 3;
pub(crate) const TYPE_HOLD: u32 = 1 << 7;
