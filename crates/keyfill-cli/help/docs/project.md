# project — установка, новая сцена, окружение

Установка в проект:

    npm i -D @keyfill/cli typescript

`@keyfill/cli` даёт команду `npx keyfill` и ставит `@keyfill/core` (API сцен, рантайм, бандлер), esbuild
и нативный бинарь под платформу (`@keyfill/cli-win32-x64`, `-linux-x64`, `-darwin-arm64`). На Intel Mac
бинаря нет — сборка из исходников: `cargo build --release -p keyfill-cli`.

Новая сцена:

    keyfill new lower-third                    # шаблон lower-third: компонент, тема, сцена, сценарий, тесты
    keyfill new scenes/title --template blank  # минимальная сцена с одним текстом

Создаёт каталог сцены. Если в проекте (ближайший каталог с `package.json`) их ещё нет —
`tsconfig.json`, `keyfill.config.ts` со сценой в `scenes`, `.gitignore` и шрифт Inter (OFL) в `fonts/inter`.
Существующие файлы не трогает. Каталог сцены годится как `<entry>`: `keyfill check lower-third`.
Дальше — `keyfill check`, `keyfill test` (первый прогон пишет снапшоты), `tsc -p .`.

Окружение:

    keyfill doctor         node 18+, @keyfill/core и его версия, esbuild, ffmpeg (для видео), конфиг,
                        typescript, кэш и пробная сборка сцены; код 1, если keyfill не соберёт сцену
    keyfill cache          каталог и размер кэша бандлов (.keyfill/cache)
    keyfill cache clean    удалить кэш; следующий запуск соберёт сцены заново

Где keyfill берёт `@keyfill/core`: `KEYFILL_CORE` (его ставит `npx keyfill`), иначе `node_modules/@keyfill/core`
вверх от каталога запуска, иначе `packages/core` репозитория keyfill.
