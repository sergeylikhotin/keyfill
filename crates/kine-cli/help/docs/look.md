# look — стиль узла

    <Box look={[theme.panel, on(live, theme.panelLive), { radius: 16 }]} />

- Значение или список слоёв; слои применяются по порядку, побеждает последний. `false`, `null`,
  `undefined` пропускаются. Состояние — `on(signal, layer)`: реактивный слой.
- Наследования нет. Исключение одно: `Span` внутри `Text` наследует текстовые свойства своего
  `Text` и переопределяет заданные.
- Для каждого значения kine помнит слой и токены темы (provenance): `kine inspect <сцена> <узел>`.

Box: `background`, `padding` (`16` или `[16, 24]` или `[t, r, b, l]`), `radius`, `border`
(`{ width, paint, align }`), `shadow` (`{ x, y, blur, spread, color, inset }`), `opacity`, `blend`,
`clip`, `transform` (`{ x, y, scale, rotate, origin }`), `transition`.

Text — всё от Box и `font`, `size`, `weight`, `color`, `lineHeight`, `tracking` (em), `align`
(`start|center|end`), `case` (`none|upper|lower`).

Span: `font`, `size`, `weight`, `color`, `tracking`, `case`.

Цвета: `hex('#RRGGBB', alpha?)`, `oklch(l, c, h, alpha?)`, токены темы.

Тема — типизированные токены:

    export const theme = defineTheme({ color: { accent: hex('#E4002B') }, font: { body: { family: 'Inter' } } });
    <Text look={{ color: theme.color.accent, font: theme.font.body }} />

Смена темы — `setTheme(theme, values)`. Токен, которого нет в значениях, — `theme.missing-token`.
