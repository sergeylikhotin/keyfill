# cli — вывод, коды выхода, общие флаги

По умолчанию вывод — компактный текст. `--json` печатает один объект со схемой `"kine": 1`;
текст строится из той же структуры, поэтому они не расходятся.

Успех:

    { "kine": 1, "command": "check", "ok": true, "exit": 1, "entry": "...", "at": { "t", "frame" },
      "result": { ... }, "logs": [...], "next": ["kine inspect ..."], "repro": "kine check ... --t 0..4.2" }

Ошибка:

    { "kine": 1, "command": "dump", "ok": false, "exit": 3,
      "error": { "kind": "usage|build|js|internal", "message": "...", "diagnostics": [...] } }

- `next` — готовые команды следующего шага; `repro` — строка, которая воспроизводит вызов
  с уже разрешённым временем.
- Диагностика: `{ level, rule, node, source: "file:line:col", message, hint, detail }`.
  Объяснение правила — `kine explain <rule>`.

Коды выхода: 0 — успех, 1 — ошибки в диагностиках (или упавшие тесты), 2 — неверный вызов,
3 — сборка, 4 — исключение JS, 5 — сбой kine.

Общие флаги команд, которые строят кадр:

    --t <время>        момент или диапазон (kine docs time)
    --props data.json  данные сцены из файла; --set name=Иван — одно значение
    --cue "0 play; 2 stop"   команды сценария в короткой форме
    --mode dev|air     dev: нарушение must — ошибка; air: применяются шаги else
    --bg "#RRGGBB"     подложка: рисуется под сценой и нужна для проверки контраста
    --seed N --now MS  сид Math.random и значение Date.now
    --json

Значения по умолчанию для `--mode`, `--bg`, `--seed`, `--now` и каталога вывода задаёт
`kine.config.ts` (kine docs config); флаги перекрывают конфиг.

Бандл кэшируется в `.kine/cache`: `KINE_CACHE_DIR` — другой каталог, `KINE_NO_CACHE=1` — без кэша.
