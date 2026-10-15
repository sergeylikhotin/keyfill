//! Текст: шрифты, шейпинг и перенос через parley, подгонка `fit`.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, Layout, LayoutContext, LineHeight, PositionedLayoutItem,
    StyleProperty,
};

use crate::diag::{Diagnostics, Level};
use crate::look::{Rgba, TextAlign, TextCase, TextLook};

/// Кисть глифа: цвет RGBA8 без умножения на альфу.
pub type Brush = [u8; 4];

#[derive(Debug, Clone)]
pub struct LoadedFont {
    pub file: String,
    pub families: Vec<String>,
}

pub struct TextEngine {
    font_cx: FontContext,
    layout_cx: LayoutContext<Brush>,
    pub system_fonts: bool,
    pub fonts: Vec<LoadedFont>,
    /// Ошибки загрузки шрифтов; build_frame переносит их в диагностики кадра.
    pub diagnostics: Diagnostics,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new(true)
    }
}

impl TextEngine {
    pub fn new(system_fonts: bool) -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts });
        Self {
            font_cx: FontContext { collection, source_cache: SourceCache::default() },
            layout_cx: LayoutContext::new(),
            system_fonts,
            fonts: Vec::new(),
            diagnostics: Diagnostics::default(),
        }
    }

    /// Движок для сцены: системные шрифты по её настройке и файлы из `fonts`.
    /// Пути абсолютные: их выдаёт бандлер из `import font from './Font.otf'`.
    pub fn for_scene(scene: &crate::Scene) -> Self {
        let Some(stage) = &scene.stage else { return Self::default() };
        let mut engine = Self::new(stage.system_fonts);
        for file in &stage.fonts {
            let shown = display_path(Path::new(file));
            match std::fs::read(file) {
                Ok(bytes) => {
                    let families = engine.register_font(bytes);
                    if families.is_empty() {
                        engine.diagnostics.push(Level::Error, "font.invalid", "stage", None, format!("{shown}: не удалось прочитать шрифт"), None);
                    }
                    engine.fonts.push(LoadedFont { file: shown, families });
                }
                Err(e) => engine.diagnostics.push(
                    Level::Error,
                    "font.file-missing",
                    "stage",
                    None,
                    format!("{shown}: {e}"),
                    Some("шрифт подключается импортом: import inter from './Inter.otf', затем defineScene({ fonts: [inter] })"),
                ),
            }
        }
        engine
    }

    /// Регистрирует шрифт и возвращает имена семейств, которые в нём нашлись.
    pub fn register_font(&mut self, bytes: Vec<u8>) -> Vec<String> {
        let registered = self.font_cx.collection.register_fonts(Blob::new(std::sync::Arc::new(bytes)), None);
        let mut names: Vec<String> = registered
            .iter()
            .filter_map(|(id, _)| self.font_cx.collection.family_name(*id).map(String::from))
            .collect();
        names.dedup();
        names
    }

    fn layout(&mut self, source: &TextSource, size: f32, truncate: Option<usize>) -> (Layout<Brush>, String) {
        let (content, ranges) = source.assemble(truncate);
        let base = &source.look;
        let scale = size / base.size;
        let mut builder = self.layout_cx.ranged_builder(&mut self.font_cx, &content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(family_stack(base.family.as_deref())));
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(base.weight)));
        builder.push_default(StyleProperty::Brush(base.color.to_u8()));
        builder.push_default(StyleProperty::LetterSpacing(base.tracking * size));
        if let Some(lh) = base.line_height {
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(lh)));
        }
        for (range, span) in ranges {
            let Some(span) = span else { continue };
            if let Some(f) = &span.family {
                builder.push(StyleProperty::FontFamily(family_stack(Some(f))), range.clone());
            }
            let span_size = span.size.map(|s| s * scale);
            if let Some(s) = span_size {
                builder.push(StyleProperty::FontSize(s), range.clone());
            }
            if let Some(w) = span.weight {
                builder.push(StyleProperty::FontWeight(FontWeight::new(w)), range.clone());
            }
            if let Some(c) = span.color {
                builder.push(StyleProperty::Brush(c.to_u8()), range.clone());
            }
            if let Some(t) = span.tracking {
                builder.push(StyleProperty::LetterSpacing(t * span_size.unwrap_or(size)), range.clone());
            }
        }
        (builder.build(&content), content)
    }

    pub fn build(&mut self, source: TextSource) -> TextBlock {
        let size = source.look.size;
        let (mut layout, content) = self.layout(&source, size, None);
        layout.break_all_lines(None);
        let resolved_family = first_family(&layout);
        TextBlock {
            layout,
            content,
            size,
            truncated: false,
            overflow: false,
            clip: false,
            resolved_family,
            source,
            fitted_for: None,
        }
    }
}

/// Путь для дампа и сообщений: относительно рабочего каталога, если файл внутри него.
fn display_path(path: &Path) -> String {
    let rel = std::env::current_dir().ok().and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf));
    rel.as_deref().unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn family_stack(family: Option<&str>) -> FontFamily<'static> {
    let css = match family {
        Some(f) => format!("\"{f}\", sans-serif"),
        None => "sans-serif".to_string(),
    };
    FontFamily::Source(Cow::Owned(css))
}

/// Переопределения стиля на диапазоне текста (`Span`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpanStyle {
    pub family: Option<String>,
    pub size: Option<f32>,
    pub weight: Option<f32>,
    pub color: Option<Rgba>,
    pub tracking: Option<f32>,
    pub case: Option<TextCase>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunSpec {
    pub text: String,
    pub style: Option<SpanStyle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitThen {
    None,
    Ellipsis,
    Clip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub shrink_to: Option<f32>,
    pub lines: Option<usize>,
    pub then: FitThen,
}

/// Диапазон байтов склеенного текста и стиль Span на нём.
type StyledRange<'a> = (Range<usize>, Option<&'a SpanStyle>);

#[derive(Debug, Clone)]
pub struct TextSource {
    pub runs: Vec<RunSpec>,
    pub look: TextLook,
    pub fit: Option<Fit>,
}

impl TextSource {
    /// Склеивает runs с учётом `case`; при `truncate` оставляет первые n символов и добавляет «…».
    fn assemble(&self, truncate: Option<usize>) -> (String, Vec<StyledRange<'_>>) {
        let mut content = String::new();
        let mut ranges = Vec::new();
        let mut left = truncate;
        for run in &self.runs {
            let case = run.style.as_ref().and_then(|s| s.case).unwrap_or(self.look.case);
            let mut text = match case {
                TextCase::None => run.text.clone(),
                TextCase::Upper => run.text.to_uppercase(),
                TextCase::Lower => run.text.to_lowercase(),
            };
            if let Some(n) = left {
                let count = text.chars().count();
                if count >= n {
                    text = text.chars().take(n).collect::<String>().trim_end().to_string() + "…";
                    left = Some(0);
                } else {
                    left = Some(n - count);
                }
            }
            let start = content.len();
            content.push_str(&text);
            ranges.push((start..content.len(), run.style.as_ref()));
            if left == Some(0) {
                break;
            }
        }
        (content, ranges)
    }

    fn char_count(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum()
    }
}

pub struct TextBlock {
    pub layout: Layout<Brush>,
    /// Итоговый текст: после `case` и, если было, обрезки.
    pub content: String,
    /// Кегль после `fit`.
    pub size: f32,
    pub truncated: bool,
    /// Текст не поместился даже после `fit`.
    pub overflow: bool,
    /// `fit.then: 'clip'` — рендер обрезает текст по контентному блоку.
    pub clip: bool,
    pub resolved_family: Option<String>,
    pub source: TextSource,
    fitted_for: Option<Option<u32>>,
}

const SLACK: f32 = 0.5;

impl TextBlock {
    pub fn look(&self) -> &TextLook {
        &self.source.look
    }

    /// Раскладывает текст под ширину. С `fit` подбирает наибольший кегль (шаг 0.5),
    /// при котором текст укладывается в `lines` строк, затем при необходимости обрезает.
    pub fn fit_to(&mut self, engine: &mut TextEngine, width: Option<f32>) {
        let key = width.map(|w| (w * 4.0).round() as u32);
        if self.fitted_for == Some(key) {
            return;
        }
        self.fitted_for = Some(key);

        let base = self.source.look.size;
        self.truncated = false;
        self.overflow = false;
        self.clip = false;

        let Some(w) = width else {
            self.relayout(engine, base, None, None);
            return;
        };
        let Some(fit) = self.source.fit.clone() else {
            self.relayout(engine, base, None, Some(w));
            self.overflow = self.layout.width() > w + SLACK;
            return;
        };

        let max_lines = fit.lines.unwrap_or(1);
        let min = fit.shrink_to.unwrap_or(base).min(base);
        if self.try_fit(engine, base, None, w, max_lines) {
            return;
        }
        // Бинарный поиск по шагам 0.5 между min и base.
        let steps = ((base - min) * 2.0).floor() as i32;
        let (mut lo, mut hi) = (0, steps);
        let mut best = None;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let size = base - mid as f32 * 0.5;
            if self.try_fit(engine, size, None, w, max_lines) {
                best = Some(mid);
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }
        if let Some(step) = best {
            self.try_fit(engine, base - step as f32 * 0.5, None, w, max_lines);
            return;
        }

        match fit.then {
            FitThen::Ellipsis => {
                let total = self.source.char_count();
                let (mut lo, mut hi, mut best) = (0usize, total, 0usize);
                while lo <= hi {
                    let mid = (lo + hi) / 2;
                    if self.try_fit(engine, min, Some(mid), w, max_lines) {
                        best = mid;
                        lo = mid + 1;
                    } else if mid == 0 {
                        break;
                    } else {
                        hi = mid - 1;
                    }
                }
                self.try_fit(engine, min, Some(best), w, max_lines);
                self.truncated = true;
            }
            FitThen::Clip => {
                self.relayout(engine, min, None, Some(w));
                self.clip = true;
            }
            FitThen::None => {
                self.relayout(engine, min, None, Some(w));
                self.overflow = true;
            }
        }
    }

    fn relayout(&mut self, engine: &mut TextEngine, size: f32, truncate: Option<usize>, width: Option<f32>) {
        let (mut layout, content) = engine.layout(&self.source, size, truncate);
        layout.break_all_lines(width.map(|w| w + SLACK));
        self.layout = layout;
        self.content = content;
        self.size = size;
    }

    fn try_fit(&mut self, engine: &mut TextEngine, size: f32, truncate: Option<usize>, w: f32, max_lines: usize) -> bool {
        self.relayout(engine, size, truncate, Some(w));
        self.layout.len() <= max_lines && self.layout.width() <= w + SLACK
    }

    /// Выравнивание по итоговой ширине; раскладку под эту ширину делает `fit_to`.
    pub fn align(&mut self) {
        let alignment = match self.source.look.align {
            TextAlign::Start => Alignment::Start,
            TextAlign::Center => Alignment::Center,
            TextAlign::End => Alignment::End,
        };
        self.layout.align(alignment, AlignmentOptions::default());
    }

    pub fn line_count(&self) -> usize {
        self.layout.len()
    }

    /// Цвета всех runs: для проверки контраста.
    pub fn colors(&self) -> Vec<Rgba> {
        let mut colors = vec![self.source.look.color];
        for run in &self.source.runs {
            if let Some(c) = run.style.as_ref().and_then(|s| s.color) {
                colors.push(c);
            }
        }
        colors
    }
}

fn first_family(layout: &Layout<Brush>) -> Option<String> {
    use skrifa::MetadataProvider;
    use skrifa::string::StringId;

    let line = layout.lines().next()?;
    let run = line.items().find_map(|item| match item {
        PositionedLayoutItem::GlyphRun(run) => Some(run),
        PositionedLayoutItem::InlineBox(_) => None,
    })?;
    let font = run.run().font();
    let font_ref = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
    // Типографское семейство (16) объединяет начертания: «Inter», а не «Inter Semi Bold».
    let name = font_ref
        .localized_strings(StringId::TYPOGRAPHIC_FAMILY_NAME)
        .english_or_first()
        .or_else(|| font_ref.localized_strings(StringId::FAMILY_NAME).english_or_first())?;
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::TextLook;

    fn engine() -> TextEngine {
        let mut e = TextEngine::new(false);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/inter");
        for f in ["Inter-Regular.otf", "Inter-SemiBold.otf"] {
            let names = e.register_font(std::fs::read(dir.join(f)).unwrap());
            assert!(names.iter().any(|n| n == "Inter"), "{names:?}");
        }
        e
    }

    fn source(text: &str, fit: Option<Fit>) -> TextSource {
        let look = TextLook {
            family: Some("Inter".into()),
            size: 40.0,
            weight: 400.0,
            color: Rgba::WHITE,
            line_height: None,
            tracking: 0.0,
            align: TextAlign::Start,
            case: TextCase::None,
        };
        TextSource { runs: vec![RunSpec { text: text.into(), style: None }], look, fit }
    }

    const LONG: &str = "Футбольный клуб Торпедо имени Эдуарда Стрельцова";

    #[test]
    fn fit_shrinks_to_one_line() {
        let mut e = engine();
        let mut b = e.build(source("Крылья Советов Самара", Some(Fit { shrink_to: Some(20.0), lines: None, then: FitThen::None })));
        b.fit_to(&mut e, Some(380.0));
        assert_eq!(b.line_count(), 1);
        assert!(b.size < 40.0 && b.size >= 20.0, "{}", b.size);
        assert!(b.layout.width() <= 380.5);
        assert!(!b.overflow);
    }

    #[test]
    fn fit_flags_overflow_when_minimum_is_not_enough() {
        let mut e = engine();
        let mut b = e.build(source(LONG, Some(Fit { shrink_to: Some(30.0), lines: Some(1), then: FitThen::None })));
        b.fit_to(&mut e, Some(300.0));
        assert!(b.overflow);
        assert_eq!(b.size, 30.0);
    }

    #[test]
    fn fit_ellipsis_truncates_and_fits() {
        let mut e = engine();
        let mut b = e.build(source(LONG, Some(Fit { shrink_to: Some(30.0), lines: Some(1), then: FitThen::Ellipsis })));
        b.fit_to(&mut e, Some(300.0));
        assert!(b.truncated && b.content.ends_with('…'), "{}", b.content);
        assert_eq!(b.line_count(), 1);
        assert!(b.layout.width() <= 300.5);
        assert!(!b.overflow);
    }

    #[test]
    fn span_case_and_ranges() {
        let mut e = engine();
        let mut src = source("", None);
        src.runs = vec![
            RunSpec { text: "live".into(), style: Some(SpanStyle { case: Some(TextCase::Upper), color: Some(Rgba::WHITE), ..Default::default() }) },
            RunSpec { text: " · 67′".into(), style: None },
        ];
        let b = e.build(src);
        assert_eq!(b.content, "LIVE · 67′");
    }
}
