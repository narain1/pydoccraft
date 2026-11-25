//! Map SVG author font names and fallback stacks into the native shaper.
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use resvg::usvg::{
    self,
    fontdb::{Database, FaceInfo, Family, Language, Query, Source, Stretch, Style, Weight, ID},
    Font, FontFamily, FontStretch, FontStyle,
};
use skrifa::{FontRef, MetadataProvider};

use super::super::{
    charge,
    css::{parse_font_family, FontFamilyValue},
    fonts::{self, ranges},
    invalid, limited, Book,
};
use crate::EngineError;

#[path = "epub_svg_font_runs.rs"]
mod runs;
#[path = "epub_svg_font_variations.rs"]
mod variations;
pub(super) use variations::CssFont;

const FONT_BYTES: usize = 16 * 1024 * 1024;
const MAX_FACES: usize = 512;
const MAX_STACKS: usize = 512;
const MAX_WORK: usize = 4 * 0x110000;
const MAX_FALLBACK_CHARACTERS: usize = 65_536;
const MAX_STACK_ENTRIES: usize = 128;
const INTERNAL_FAMILY: &str = "\0EPUB resolved SVG face";

#[derive(Default)]
pub(super) struct Families {
    // Authored names and their private SVG family identifiers.
    names: HashMap<String, String>,
    variant_names: HashSet<String>,
    requests: HashMap<CssFont, String>,
}

impl Families {
    pub(super) fn new(faces: &[fonts::Face]) -> Self {
        Self {
            variant_names: faces
                .iter()
                .filter(|face| variations::has_descriptors(face))
                .map(|face| face.family.to_lowercase())
                .collect(),
            ..Self::default()
        }
    }

    pub(super) fn tracks_variations(&self) -> bool {
        !self.variant_names.is_empty()
    }

    pub(super) fn variant(&mut self, request: &CssFont) -> Result<Option<String>, EngineError> {
        if !request
            .family
            .split(',')
            .any(|name| name.starts_with("doccraft-svg-variable-"))
        {
            return Ok(None);
        }
        let next = self.requests.len();
        if next == MAX_STACKS && !self.requests.contains_key(request) {
            return Err(limited(
                "EPUB SVG font selection exceeds 512 distinct stacks",
            ));
        }
        let alias = self
            .requests
            .entry(request.clone())
            .or_insert_with(|| format!("doccraft-svg-request-{next}"));
        Ok(Some(alias.clone()))
    }

    pub(super) fn encode(&mut self, value: &str) -> Result<Option<String>, EngineError> {
        let Some(value) = parse_font_family(value) else {
            return Ok(None);
        };
        match value {
            FontFamilyValue::Inherit => Ok(Some("inherit".into())),
            FontFamilyValue::Initial => Ok(Some("serif".into())),
            FontFamilyValue::Families(families) => {
                if families.len() > MAX_STACK_ENTRIES {
                    return Err(limited("EPUB SVG font stack exceeds 128 families"));
                }
                let mut encoded = Vec::with_capacity(families.len());
                for family in families.iter() {
                    if family.generic {
                        encoded.push(match family.name.as_str() {
                            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" => {
                                family.name.clone()
                            }
                            "ui-serif" => "serif".into(),
                            "ui-monospace" => "monospace".into(),
                            _ => "sans-serif".into(),
                        });
                    } else {
                        let next = self.names.len();
                        let name = family.name.to_lowercase();
                        let prefix = if self.variant_names.contains(&name) {
                            "doccraft-svg-variable-"
                        } else {
                            "doccraft-svg-font-"
                        };
                        let alias = self
                            .names
                            .entry(name)
                            .or_insert_with(|| format!("{prefix}{next}"));
                        encoded.push(alias.clone());
                    }
                }
                Ok(Some(encoded.join(",")))
            }
        }
    }
}

struct SystemFonts {
    db: Arc<Database>,
    names: HashMap<String, String>,
    ids: Vec<ID>,
}

fn system_fonts() -> &'static SystemFonts {
    static FONTS: OnceLock<SystemFonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        for &font in wordcraft_fonts::bundled() {
            db.load_font_source(Source::Binary(Arc::new(font)));
        }
        db.set_serif_family("Source Serif 4");
        db.set_sans_serif_family("Source Sans 3");
        db.set_monospace_family("JetBrains Mono");
        let mut names = HashMap::new();
        for face in db.faces() {
            for (name, _) in &face.families {
                names
                    .entry(name.to_lowercase())
                    .or_insert_with(|| name.clone());
            }
        }
        let ids = db.faces().map(|face| face.id).collect();
        SystemFonts {
            names,
            ids,
            db: Arc::new(db),
        }
    })
}

pub(super) struct Resolver(Arc<Mutex<Selection>>);

impl Resolver {
    pub(super) fn install(
        options: &mut usvg::Options<'_>,
        faces: &[fonts::Face],
        families: Families,
        book: &mut Book<'_>,
    ) -> Result<Self, EngineError> {
        let mut selection = Selection::default();
        let aliases = families.names;
        selection.requests = families
            .requests
            .into_iter()
            .map(|(request, name)| (name, request))
            .collect();
        options.fontdb = system_fonts().db.clone();
        let mut decoded = 0;
        let mut total = 0;
        // Rules with equal descriptors are tried in reverse stylesheet order.
        for face in faces.iter().rev() {
            let Some(alias) = aliases.get(&face.family.to_lowercase()) else {
                continue;
            };
            let [weight, _] = face.weight.unwrap_or([400.0; 2]);
            let style = face.style;
            let stretch = match face.width.map(|bounds| bounds[0]) {
                Some(width) if width <= 50.0 => Stretch::UltraCondensed,
                Some(width) if width <= 62.5 => Stretch::ExtraCondensed,
                Some(width) if width <= 75.0 => Stretch::Condensed,
                Some(width) if width <= 87.5 => Stretch::SemiCondensed,
                Some(width) if width >= 200.0 => Stretch::UltraExpanded,
                Some(width) if width >= 150.0 => Stretch::ExtraExpanded,
                Some(width) if width >= 125.0 => Stretch::Expanded,
                Some(width) if width >= 112.5 => Stretch::SemiExpanded,
                _ => Stretch::Normal,
            };
            let data = fonts::read_face(face, book)?;
            charge(
                &mut decoded,
                data.len(),
                FONT_BYTES,
                "EPUB SVG decoded fonts exceed 16 MiB",
            )?;
            let database = Arc::make_mut(&mut options.fontdb);
            let ids = selection.author.entry(alias.clone()).or_default();
            let style = match style {
                parley::FontStyle::Normal => Style::Normal,
                parley::FontStyle::Italic => Style::Italic,
                parley::FontStyle::Oblique(_) => Style::Oblique,
            };
            let mut register = |data: Vec<u8>| {
                charge(
                    &mut selection.font_bytes,
                    data.len(),
                    FONT_BYTES,
                    "EPUB SVG registered fonts exceed 16 MiB",
                )?;
                let loaded = database.load_font_source(Source::Binary(Arc::new(data)));
                if loaded.is_empty() {
                    return Err(invalid("EPUB SVG font-face has no usable font"));
                }
                charge(
                    &mut total,
                    loaded.len(),
                    MAX_FACES,
                    "EPUB SVG font face count exceeds 512",
                )?;
                for id in loaded {
                    let mut info = face_info(database, id)?;
                    let request = database
                        .with_face_data(id, |data, index| {
                            FontRef::from_index(data, index)
                                .map(|font| variations::Face::new(face, font))
                                .map_err(invalid)
                        })
                        .ok_or_else(|| invalid("missing EPUB SVG font data"))??;
                    database.remove_face(id);
                    info.families = vec![(alias.clone(), Language::English_UnitedStates)];
                    info.style = style;
                    info.weight = Weight(weight as u16);
                    info.stretch = stretch;
                    let id = database.push_face_info(info);
                    selection.variants.insert(id, request);
                    ids.push(id);
                }
                Ok(())
            };
            match face.unicode_ranges.as_deref() {
                None | Some([(0, 0x10ffff)]) => register(data)?,
                Some(ranges) => {
                    ranges::register(&data, ranges, &mut selection.rewrite_work, register)?
                }
            }
        }
        selection.names = aliases
            .into_iter()
            .map(|(name, alias)| (alias, name))
            .collect();

        let state = Arc::new(Mutex::new(selection));
        let select = state.clone();
        let fallback = state.clone();
        options.font_resolver = usvg::FontResolver {
            select_font: Box::new(move |font, db| {
                let mut selector = select.lock().ok()?;
                selector.run(|selector| selector.primary(font, db))
            }),
            select_fallback: Box::new(move |ch, used, db| {
                let mut selector = fallback.lock().ok()?;
                selector.run(|selector| selector.fallback(ch, used, db))
            }),
        };
        Ok(Self(state))
    }

    pub(super) fn check(&self) -> Result<(), EngineError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| invalid("EPUB SVG font selection failed"))?;
        state.error.take().map_or(Ok(()), Err)
    }
}

#[derive(Default)]
struct Selection {
    names: HashMap<String, String>,
    author: HashMap<String, Vec<ID>>,
    requests: HashMap<String, CssFont>,
    variants: HashMap<ID, variations::Face>,
    stacks: HashMap<Vec<ID>, ID>,
    candidates: HashMap<ID, Vec<ID>>,
    family_plans: HashMap<String, Option<ID>>,
    attempted: HashMap<(ID, char), ID>,
    direct_fonts: HashMap<ID, String>,
    direct_names: HashMap<String, ID>,
    coverage: HashMap<ID, Arc<[u32]>>,
    restricted: HashMap<(ID, Vec<ID>), ID>,
    comparison_work: usize,
    rewrite_work: usize,
    font_bytes: usize,
    error: Option<EngineError>,
}

impl Selection {
    fn run(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<Option<ID>, EngineError>,
    ) -> Option<ID> {
        if self.error.is_some() {
            return None;
        }
        match operation(self) {
            Ok(face) => face,
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    fn primary(&mut self, font: &Font, db: &mut Arc<Database>) -> Result<Option<ID>, EngineError> {
        let request = if let [FontFamily::Named(name)] = font.families() {
            if let Some(&id) = self.direct_names.get(name) {
                return Ok(Some(id));
            }
            self.requests.get(name)
        } else {
            None
        };
        let planned = request.map(CssFont::families);
        let families = planned.as_deref().unwrap_or_else(|| font.families());
        charge(
            &mut self.comparison_work,
            (families.len() + 1).saturating_mul(db.len()),
            MAX_WORK,
            "EPUB SVG font matching exceeds its work budget",
        )?;
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let query = Query {
            weight: Weight(font.weight()),
            style: match font.style() {
                FontStyle::Normal => Style::Normal,
                FontStyle::Italic => Style::Italic,
                FontStyle::Oblique => Style::Oblique,
            },
            stretch: match font.stretch() {
                FontStretch::UltraCondensed => Stretch::UltraCondensed,
                FontStretch::ExtraCondensed => Stretch::ExtraCondensed,
                FontStretch::Condensed => Stretch::Condensed,
                FontStretch::SemiCondensed => Stretch::SemiCondensed,
                FontStretch::Normal => Stretch::Normal,
                FontStretch::SemiExpanded => Stretch::SemiExpanded,
                FontStretch::Expanded => Stretch::Expanded,
                FontStretch::ExtraExpanded => Stretch::ExtraExpanded,
                FontStretch::UltraExpanded => Stretch::UltraExpanded,
            },
            ..Default::default()
        };
        for family in families.iter().chain([&FontFamily::Serif]) {
            if let (Some(request), FontFamily::Named(name)) = (request, family) {
                if let Some(faces) = self.author.get(name) {
                    let ranked = faces
                        .iter()
                        .filter_map(|id| Some((*id, request.rank(self.variants.get(id)?))));
                    let best = ranked
                        .clone()
                        .map(|(_, rank)| rank)
                        .min_by(variations::rank_cmp);
                    if let Some(best) = best {
                        candidates.extend(ranked.filter_map(|(id, rank)| {
                            (variations::rank_cmp(&rank, &best).is_eq() && seen.insert(id))
                                .then_some(id)
                        }));
                    }
                    continue;
                }
            }
            let family_name = match family {
                FontFamily::Named(name) if self.author.contains_key(name) => Family::Name(name),
                FontFamily::Named(name) => {
                    let key = self
                        .names
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_lowercase());
                    let Some(name) = system_fonts().names.get(&key) else {
                        continue;
                    };
                    Family::Name(name)
                }
                FontFamily::Serif => Family::Serif,
                FontFamily::SansSerif => Family::SansSerif,
                FontFamily::Cursive => Family::Cursive,
                FontFamily::Fantasy => Family::Fantasy,
                FontFamily::Monospace => Family::Monospace,
            };
            let family_query = [family_name];
            let matched = db.query(&Query {
                families: &family_query,
                ..query
            });
            let Some(matched) = matched else { continue };
            if let FontFamily::Named(name) = family {
                if let Some(faces) = self.author.get(name) {
                    let best = face_info(db, matched)?;
                    candidates.extend(faces.iter().copied().filter(|id| {
                        db.face(*id)
                            .is_some_and(|face| same_descriptors(face, &best))
                            && seen.insert(*id)
                    }));
                    continue;
                }
            }
            if seen.insert(matched) {
                candidates.push(matched);
            }
        }
        let Some(&first) = candidates.first() else {
            return Ok(None);
        };
        let base = if let Some(&base) = self.stacks.get(&candidates) {
            base
        } else {
            if self.stacks.len() >= MAX_STACKS {
                return Err(limited(
                    "EPUB SVG font selection exceeds 512 distinct stacks",
                ));
            }
            // The fallback callback receives the primary face ID, without its family list.
            let info = face_info(db, first)?;
            let base = add_private_face(db, info);
            self.candidates.insert(base, candidates.clone());
            self.stacks.insert(candidates, base);
            base
        };
        let key = font
            .families()
            .iter()
            .map(|family| match family {
                FontFamily::Named(name) => name.clone(),
                _ => family.to_string(),
            })
            .collect::<Vec<_>>()
            .join(",");
        if self.family_plans.len() < MAX_STACKS || self.family_plans.contains_key(&key) {
            self.family_plans
                .entry(key)
                .and_modify(|id| {
                    if *id != Some(base) {
                        *id = None;
                    }
                })
                .or_insert(Some(base));
        }
        Ok(Some(base))
    }

    fn fallback(
        &mut self,
        ch: char,
        used: &[ID],
        db: &mut Arc<Database>,
    ) -> Result<Option<ID>, EngineError> {
        let Some(&base) = used.first() else {
            return Ok(None);
        };
        let Some(candidates) = self.candidates.get(&base).cloned() else {
            return Ok(None);
        };
        let code = u32::from(ch);
        for (ordinal, &candidate) in candidates.iter().enumerate().skip(1) {
            if let Some(id) = self.choose(candidate, code, &candidates[..ordinal], used, db)? {
                self.remember_fallback(base, ch, candidate)?;
                return Ok(Some(id));
            }
        }
        let best = face_info(db, base)?;
        let mut preferred = candidates;
        preferred.extend_from_slice(&used[1..]);
        for &candidate in &system_fonts().ids {
            charge(
                &mut self.comparison_work,
                1,
                MAX_WORK,
                "EPUB SVG font matching exceeds its work budget",
            )?;
            if preferred.contains(&candidate) {
                continue;
            }
            let Some(face) = db.face(candidate) else {
                continue;
            };
            if face.style != best.style
                && face.weight != best.weight
                && face.stretch != best.stretch
            {
                continue;
            }
            if let Some(id) = self.choose(candidate, code, &preferred, used, db)? {
                self.remember_fallback(base, ch, candidate)?;
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn remember_fallback(&mut self, base: ID, ch: char, candidate: ID) -> Result<(), EngineError> {
        if self.attempted.len() >= MAX_FALLBACK_CHARACTERS
            && !self.attempted.contains_key(&(base, ch))
        {
            return Err(limited(
                "EPUB SVG fallback exceeds 65536 distinct character requests",
            ));
        }
        self.attempted.insert((base, ch), candidate);
        Ok(())
    }

    fn choose(
        &mut self,
        candidate: ID,
        code: u32,
        preferred: &[ID],
        used: &[ID],
        db: &mut Arc<Database>,
    ) -> Result<Option<ID>, EngineError> {
        charge(
            &mut self.comparison_work,
            1,
            MAX_WORK,
            "EPUB SVG font fallback exceeds its work budget",
        )?;
        if used.contains(&candidate) || !self.supports(db, candidate, code) {
            return Ok(None);
        }
        let key = (candidate, preferred.to_vec());
        if let Some(&id) = self.restricted.get(&key) {
            return Ok((!used.contains(&id) && self.supports(db, id, code)).then_some(id));
        }
        let covered = preferred
            .iter()
            .map(|&id| self.characters(id, db))
            .collect::<Result<Vec<_>, _>>()?;
        let eligible = |cp| {
            covered
                .iter()
                .all(|chars| chars.binary_search(&cp).is_err())
        };
        if !eligible(code) {
            return Ok(None);
        }
        let original = self.characters(candidate, db)?;
        charge(
            &mut self.comparison_work,
            original.len().saturating_mul(covered.len()),
            MAX_WORK,
            "EPUB SVG font fallback exceeds its work budget",
        )?;
        let id = if original.iter().copied().all(eligible) {
            candidate
        } else {
            if self.restricted.len() >= MAX_FACES {
                return Err(limited("EPUB SVG font fallback exceeds 512 face variants"));
            }
            // The native fallback shaper can replace an entire run if the new face matches it.
            // Remove higher-priority characters so a later family only fills uncovered glyphs.
            let data = db
                .with_face_data(candidate, |data, index| {
                    ranges::filter_face(data, index, eligible, &mut self.rewrite_work)
                })
                .ok_or_else(|| invalid("missing EPUB SVG fallback font data"))??
                .ok_or_else(|| invalid("EPUB SVG fallback font lost its character map"))?;
            charge(
                &mut self.font_bytes,
                data.len(),
                FONT_BYTES,
                "EPUB SVG registered fonts exceed 16 MiB",
            )?;
            let mut info = face_info(db, candidate)?;
            info.source = Source::Binary(Arc::new(data));
            info.index = 0;
            add_private_face(db, info)
        };
        if self.restricted.len() < MAX_FACES {
            self.restricted.insert(key, id);
        }
        Ok(Some(id))
    }

    fn supports(&self, db: &Database, id: ID, code: u32) -> bool {
        self.coverage.get(&id).map_or_else(
            || has_character(db, id, code),
            |map| map.binary_search(&code).is_ok(),
        )
    }

    fn characters(&mut self, id: ID, db: &Database) -> Result<Arc<[u32]>, EngineError> {
        if let Some(chars) = self.coverage.get(&id) {
            return Ok(chars.clone());
        }
        let mut covered = db
            .with_face_data(id, |data, index| {
                let font = FontRef::from_index(data, index).map_err(invalid)?;
                let mut codes = Vec::new();
                for (cp, glyph) in font.charmap().mappings() {
                    charge(
                        &mut self.comparison_work,
                        1,
                        MAX_WORK,
                        "EPUB SVG font character map work limit exceeded",
                    )?;
                    if glyph.to_u32() != 0 {
                        codes.push(cp);
                    }
                }
                Ok::<_, EngineError>(codes)
            })
            .ok_or_else(|| invalid("missing EPUB SVG font data"))??;
        covered.sort_unstable();
        let covered: Arc<[u32]> = covered.into();
        self.coverage.insert(id, covered.clone());
        Ok(covered)
    }
}

fn same_descriptors(face: &FaceInfo, best: &FaceInfo) -> bool {
    face.style == best.style && face.weight == best.weight && face.stretch == best.stretch
}

fn has_character(db: &Database, id: ID, code: u32) -> bool {
    db.with_face_data(id, |data, index| {
        FontRef::from_index(data, index).is_ok_and(|font| {
            font.charmap()
                .map(code)
                .is_some_and(|glyph| glyph.to_u32() != 0)
        })
    })
    .unwrap_or_default()
}

fn face_info(db: &Database, id: ID) -> Result<FaceInfo, EngineError> {
    db.face(id)
        .cloned()
        .ok_or_else(|| invalid("missing EPUB SVG font face"))
}

fn add_private_face(db: &mut Arc<Database>, mut info: FaceInfo) -> ID {
    info.families = vec![(INTERNAL_FAMILY.into(), Language::English_UnitedStates)];
    Arc::make_mut(db).push_face_info(info)
}
