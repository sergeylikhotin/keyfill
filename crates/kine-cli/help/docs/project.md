# project — установка, новая сцена, окружение

Установка в проект:

    npm i -D @kine/cli typescript

`@kine/cli` даёт команду `npx kine` и ставит `@kine/core` (API сцен, рантайм, бандлер), esbuild
и нативный бинарь под платформу (`@kine/cli-win32-x64`, `-linux-x64`, `-darwin-arm64`, `-darwin-x64`).

Новая сцена:

    kine new lower-third                    # шаблон lower-third: компонент, тема, сцена, сценарий, тесты
    kine new scenes/title --template blank  # минимальная сцена с одним текстом

Создаёт каталог сцены. Если в проекте (ближайший каталог с `package.json`) их ещё нет —
`tsconfig.json`, `kine.config.ts` со сценой в `scenes`, `.gitignore` и шрифт Inter (OFL) в `fonts/inter`.
Существующие файлы не трогает. Каталог сцены годится как `<entry>`: `kine check lower-third`.
Дальше — `kine check`, `kine test` (первый прогон пишет снапшоты), `tsc -p .`.

Окружение:

    kine doctor         node 18+, @kine/core и его версия, esbuild, ffmpeg (для видео), конфиг,
                        typescript, кэш и пробная сборка сцены; код 1, если kine не соберёт сцену
    kine cache          каталог и размер кэша бандлов (.kine/cache)
    kine cache clean    удалить кэш; следующий запуск соберёт сцены заново

Где kine берёт `@kine/core`: `KINE_CORE` (его ставит `npx kine`), иначе `node_modules/@kine/core`
вверх от каталога запуска, иначе `packages/core` репозитория kine.
