use crate::node::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub from: BucketId,
    pub into: BucketId,
    pub ops: Vec<(Property, FilterOp)>,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            from: default(),
            into: default(),
            ops: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Property {
    Title,
    Subtitle,
    Artist,
    TitleTranslit,
    SubtitleTranslit,
    ArtistTranslit,
    Genre,
    Credit,
    Banner,
    Background,
    Lyrics,
    CdTitle,
    Music,
    Offset,
    SampleStart,
    SampleLength,
    Gamemode,
    Desc,
    Difficulty,
    Meter,
}
impl Property {
    fn get<'a>(&self, sm: &'a Simfile) -> Cow<'a, str> {
        use Property::*;
        match self {
            Title => Cow::Borrowed(&sm.title),
            Subtitle => Cow::Borrowed(&sm.subtitle),
            Artist => Cow::Borrowed(&sm.artist),
            TitleTranslit => Cow::Borrowed(&sm.title_trans),
            SubtitleTranslit => Cow::Borrowed(&sm.subtitle_trans),
            ArtistTranslit => Cow::Borrowed(&sm.artist_trans),
            Genre => Cow::Borrowed(&sm.genre),
            Credit => Cow::Borrowed(&sm.credit),
            Banner => sm
                .banner
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
            Background => sm
                .background
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
            Lyrics => sm
                .lyrics
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
            CdTitle => sm
                .cdtitle
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
            Music => sm
                .music
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
            Offset => Cow::Owned(sm.offset.to_string()),
            SampleStart => Cow::Owned(sm.sample_start.unwrap_or(0.).to_string()),
            SampleLength => Cow::Owned(sm.sample_len.unwrap_or(0.).to_string()),
            Gamemode => Cow::Owned(format!("{:?}", sm.gamemode)),
            Desc => Cow::Borrowed(&sm.desc),
            Difficulty => Cow::Owned(format!("{:?}", sm.difficulty)),
            Meter => Cow::Owned(sm.difficulty_num.to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FilterOp {
    Allow(Vec<String>),
    Deny(Vec<String>),
    LessThan(String),
    GreaterThan(String),
    Not(Box<FilterOp>),
    And(Vec<FilterOp>),
    Or(Vec<FilterOp>),
}
impl FilterOp {
    pub fn matches(&self, val: &str) -> bool {
        use FilterOp::*;
        match self {
            Allow(whitelist) => whitelist
                .iter()
                .any(|w| natord::compare_ignore_case(w, val) == cmp::Ordering::Equal),
            Deny(blacklist) => !blacklist
                .iter()
                .any(|w| natord::compare_ignore_case(w, val) == cmp::Ordering::Equal),
            LessThan(top) => natord::compare_ignore_case(val, top) == cmp::Ordering::Less,
            GreaterThan(top) => natord::compare_ignore_case(val, top) == cmp::Ordering::Greater,
            Not(op) => !op.matches(val),
            And(ops) => ops.iter().all(|op| op.matches(val)),
            Or(ops) => ops.iter().any(|op| op.matches(val)),
        }
    }
}

impl Node for Filter {
    fn apply(&self, store: &mut SimfileStore) -> Result<()> {
        store.get(&self.from, |store, mut list| {
            list.retain(|sm| self.ops.iter().all(|(prop, op)| op.matches(&*prop.get(sm))));
            store.put(&self.into, list);
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
