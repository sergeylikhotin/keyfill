//! Текст: шейпинг и перенос строк через parley.

use std::borrow::Cow;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, Layout, LayoutContext, LineHeight, PositionedLayoutItem,
    StyleProperty,
};

use crate::look::{TextAlign, TextCase, TextLook};

/// Кисть глифа: цвет RGBA8 без умножения на альфу.
pub type Brush = [u8; 4];

pub struct TextEngine {
    font_cx: FontContext,
    layout_cx: LayoutContext<Brush>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self { font_cx: FontContext::new(), layout_cx: LayoutContext::new() }
    }
}

pub struct TextBlock {
    pub layout: Layout<Brush>,
    pub look: TextLook,
    /// Текст после применения `case`.
    pub content: String,
    /// Семейство, которое реально использовано для первого прогона глифов.
    pub resolved_family: Option<String>,
}

impl TextEngine {
    pub fn build(&mut self, content: &str, look: &TextLook) -> TextBlock {
        let content = match look.case {
            TextCase::None => content.to_string(),
            TextCase::Upper => content.to_uppercase(),
            TextCase::Lower => content.to_lowercase(),
        };
        let family = match &look.family {
            Some(f) => format!("\"{f}\", sans-serif"),
            None => "sans-serif".to_string(),
        };

        let mut builder = self.layout_cx.ranged_builder(&mut self.font_cx, &content, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(family))));
        builder.push_default(StyleProperty::FontSize(look.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(look.weight)));
        builder.push_default(StyleProperty::Brush(look.color.to_u8()));
        builder.push_default(StyleProperty::LetterSpacing(look.tracking * look.size));
        if let Some(lh) = look.line_height {
            builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(lh)));
        }
        let mut layout: Layout<Brush> = builder.build(&content);
        layout.break_all_lines(None);

        let resolved_family = first_family(&layout);
        TextBlock { layout, look: look.clone(), content, resolved_family }
    }
}

impl TextBlock {
    /// Перенос и выравнивание под итоговую ширину контентного блока.
    pub fn finalize(&mut self, width: f32) {
        // Небольшой допуск: ширина из measure могла округлиться и спровоцировать лишний перенос.
        self.layout.break_all_lines(Some(width + 0.5));
        let alignment = match self.look.align {
            TextAlign::Start => Alignment::Start,
            TextAlign::Center => Alignment::Center,
            TextAlign::End => Alignment::End,
        };
        self.layout.align(alignment, AlignmentOptions::default());
    }

    pub fn line_count(&self) -> usize {
        self.layout.len()
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
    let name = font_ref.localized_strings(StringId::FAMILY_NAME).english_or_first()?;
    Some(name.to_string())
}
