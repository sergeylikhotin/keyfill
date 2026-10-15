//! Золотой дамп эталонного примера. Обновить: `KINE_UPDATE_SNAPSHOTS=1 cargo test -p kine-cli`.
//!
//! Дамп включает имя реально использованного шрифта, поэтому снапшот привязан к набору
//! системных шрифтов машины. Шрифты из файлов (M1) уберут эту зависимость.

use std::path::Path;

use kine_js::RunOptions;

#[test]
fn lower_third_dump_matches_snapshot() {
    let home = std::path::absolute(kine_cli::kine_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return;
    }
    // Пути в дампе относительны рабочего каталога, снапшот снят из корня репозитория.
    std::env::set_current_dir(&home).unwrap();

    let entry = Path::new("examples/lower-third/scene.tsx");
    let first = kine_cli::frame(entry, &RunOptions::default()).unwrap();
    let dump = kine_core::dump::dump(&first.scene, &first.frame);

    let second = kine_cli::frame(entry, &RunOptions::default()).unwrap();
    assert_eq!(dump, kine_core::dump::dump(&second.scene, &second.frame), "дамп недетерминирован");

    let snapshot = home.join("examples/lower-third/scene.dump.txt");
    if std::env::var_os("KINE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snapshot, &dump).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&snapshot).expect("нет снапшота: запусти с KINE_UPDATE_SNAPSHOTS=1");
    assert_eq!(dump.replace("\r\n", "\n"), expected.replace("\r\n", "\n"));
}
