# layout — раскладка, якоря, fit

Внутри контейнеров — flex и grid (Taffy):

    <Box layout={row({ gap: 12, align: 'center', justify: 'between' })}>…</Box>
    <Box layout={column({ gap: 8 })}>…</Box>
    <Box layout={grid({ columns: [{ fr: 1 }, 120], rows: ['auto'], gap: [12, 8] })}>
      <Text cell={{ col: 2, row: 1, colSpan: 1 }}>…</Text>
    </Box>
    <Box layout={stack()}>…</Box>          // дети накладываются, каждый ставится anchor

Дорожки grid: число, `pct(n)`, `{ fr: n }`, `'auto'`. Ячейки с 1; без `cell` — по порядку.

Размер: `size={{ w: 640, h: 'auto' }}`, `min`, `max`, `grow`, `shrink`, `aspect`.

Якорь ставит узел относительно цели:

    anchor={{ to: 'titleSafe', at: 'bottom-left', self?: 'bottom-left', offset?: [0, -24] }}

`to`: `stage`, `actionSafe`, `titleSafe`, `parent` (по умолчанию) или часть компонента
(`part.status`). Узел верхнего уровня раскладывается в пределах цели; узел внутри контейнера
выносится из потока. Внутри `stack()` якорь к родителю — ячейка с выравниванием.

Текст, который должен поместиться:

    <Text fit={{ shrinkTo: 32, lines: 2, then: 'ellipsis' }}>{name}</Text>

keyfill подбирает наибольший кегль с шагом 0.5 от `size` до `shrinkTo`, при котором текст укладывается
в `lines` строк (по умолчанию 1). Не вышло — `then: 'ellipsis'` обрезает с «…», `'clip'` — по блоку,
без `then` текст помечен непоместившимся, и его ловит `must.fit`. В дампе: `(fit 34→30.5)`.

`when={live}` — узел монтируется по условию: при false уходит через свой `out`.
