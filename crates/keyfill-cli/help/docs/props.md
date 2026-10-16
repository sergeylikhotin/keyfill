# props — типы пропсов и данных

    import { component, prop } from '@keyfill/core';

    component({
      name: 'Scoreboard',
      props: {
        home: String,                                  // коротко: String, Number, Boolean
        side: prop.enum(['home', 'away']),             // одно из значений
        score: prop.number({ min: 0, int: true, default: 0 }),
        players: prop.list(prop.object({ name: String, number: prop.number({ default: 0 }) }), { default: [] }),
        logo: prop.image({ description: 'Логотип команды' }),
        extra: prop.json<{ a: number }>(),             // любой JSON: только для tsc
      },
      ...
    });

Из одного описания получаются три вещи:

- **TS-тип.** Внутри компонента `p.side()` — `'home' | 'away'`, `p.players()[0].number` — `number`.
  Пропс с `default` снаружи можно не передавать (и поле объекта с `default` тоже).
- **JSON Schema** для пульта и агента: `keyfill schema <файл> --json`, поле `schema` у каждого
  пропса и параметра команды. По ней пульт строит форму: выпадающий список для `enum`, число
  с границами, выбор файла для `image` (`"x-keyfill": "image"`).
- **Проверка значений** из `--props`/`--set`, `update` и `invoke` сценария и команд канала:
  ошибка называет путь — `players[1].number: ожидается число, получено "7"`.

Типы: `prop.string`, `prop.number({ min, max, int })`, `prop.boolean`, `prop.enum([...])`,
`prop.list(item, { min, max })`, `prop.object({ ... })`, `prop.image()`, `prop.json<T>()`. У всех
есть `default` и `description`. Параметры команд (`commands.x.params`) описываются так же.

`prop.json<T>()` — запасной выход: keyfill значение не проверяет, а пульт показывает его как
сырой JSON. Если тип выражается через `prop.*`, лучше описать его.
