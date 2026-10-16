//! `kine new <имя> [--template lower-third|blank]`: каталог сцены из шаблона и, если их нет,
//! файлы проекта — `tsconfig.json`, `kine.config.ts`, `.gitignore` и шрифт Inter (OFL).
//!
//! Шаблоны встроены в бинарь и совпадают с его версией. Корень проекта — ближайший каталог
//! с `package.json` вверх от каталога запуска, без него — сам каталог запуска.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Serialize;

use crate::slash;

pub const TEMPLATES: &[&str] = &["lower-third", "blank"];

/// Файлы шаблона: имя в каталоге сцены и текст. `Component.tsx` получает имя компонента.
fn template(name: &str) -> Option<Vec<(&'static str, &'static str)>> {
    match name {
        "lower-third" => Some(vec![
            ("Component.tsx", include_str!("../templates/lower-third/Component.tsx")),
            ("theme.ts", include_str!("../templates/lower-third/theme.ts")),
            ("scene.tsx", include_str!("../templates/lower-third/scene.tsx")),
            ("scene.cues.ts", include_str!("../templates/lower-third/scene.cues.ts")),
            ("test.ts", include_str!("../templates/lower-third/test.ts")),
        ]),
        "blank" => Some(vec![
            ("scene.tsx", include_str!("../templates/blank/scene.tsx")),
            ("test.ts", include_str!("../templates/blank/test.ts")),
        ]),
        _ => None,
    }
}

const FONTS: &[(&str, &[u8])] = &[
    ("Inter-Regular.otf", include_bytes!("../../../assets/fonts/inter/Inter-Regular.otf")),
    ("Inter-SemiBold.otf", include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.otf")),
    ("OFL.txt", include_bytes!("../../../assets/fonts/inter/OFL.txt")),
];

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "jsxImportSource": "@kine/core",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true
  },
  "include": ["**/*.ts", "**/*.tsx"],
  "exclude": ["node_modules", "out", ".kine"]
}
"#;

const GITIGNORE: &str = "node_modules/\nout/\n.kine/\n";

#[derive(Debug, Serialize)]
pub struct Created {
    pub template: String,
    /// Каталог сцены.
    pub dir: String,
    /// Корень проекта: там `tsconfig.json`, `kine.config.ts`, `fonts/`.
    pub root: String,
    pub component: Option<String>,
    /// Созданные файлы.
    pub files: Vec<String>,
    /// Файлы проекта, которые уже были и не тронуты.
    pub kept: Vec<String>,
    /// Точка входа для команд: сценарий или сцена.
    pub entry: String,
}

/// Имя компонента из имени сцены: `lower-third` → `LowerThird`. Не латиница — имя шаблона.
fn component_name(name: &str, fallback: &str) -> String {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let out: String = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) { out } else { fallback.to_string() }
}

/// Путь от `from` к `to` для импорта: `../fonts/inter`.
fn relative(from: &Path, to: &Path) -> String {
    let a: Vec<_> = from.components().collect();
    let b: Vec<_> = to.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".into(); a.len() - common];
    parts.extend(b[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
    let s = parts.join("/");
    if s.starts_with("..") { s } else { format!("./{s}") }
}

pub fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors().find(|d| d.join("package.json").is_file()).unwrap_or(cwd).to_path_buf()
}

pub fn create(name: &str, template_name: &str) -> Result<Created> {
    let files = template(template_name).ok_or_else(|| crate::usage(format!("шаблона {template_name} нет; есть: {}", TEMPLATES.join(", "))))?;
    let cwd = std::env::current_dir()?;
    let dir = cwd.join(name);
    if dir.exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(crate::usage(format!("{} уже есть и не пуст; выбери другое имя", slash(&shown(&cwd, &dir)))));
    }
    let root = project_root(&cwd);
    let fonts_dir = root.join("fonts/inter");
    let component = component_name(name, if template_name == "blank" { "Scene" } else { "LowerThird" });
    let slug = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "scene".into());
    let has_cues = files.iter().any(|(f, _)| *f == "scene.cues.ts");
    let entry = dir.join(if has_cues { "scene.cues.ts" } else { "scene.tsx" });
    let entry_shown = slash(&shown(&cwd, &entry));

    let mut created = Vec::new();
    let mut kept = Vec::new();
    let write = |path: PathBuf, bytes: &[u8], created: &mut Vec<String>| -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("не удалось создать {}", parent.display()))?;
        }
        std::fs::write(&path, bytes).with_context(|| format!("не удалось записать {}", path.display()))?;
        created.push(slash(&shown(&cwd, &path)));
        Ok(())
    };

    for (file, text) in &files {
        let target = match *file {
            "Component.tsx" => format!("{component}.tsx"),
            "test.ts" => format!("{slug}{}", crate::suite::SUFFIX),
            other => other.to_string(),
        };
        let text = text
            .replace("__Name__", &component)
            .replace("__FONTS__", &relative(&dir, &fonts_dir))
            .replace("__entry__", &slash(&shown(&cwd, &dir.join("scene.tsx"))))
            .replace("__cues__", &entry_shown)
            .replace("__dir__", &slash(&shown(&cwd, &dir)));
        write(dir.join(target), text.as_bytes(), &mut created)?;
    }

    for (font, bytes) in FONTS {
        let path = fonts_dir.join(font);
        if path.exists() {
            kept.push(slash(&shown(&cwd, &path)));
        } else {
            write(path, bytes, &mut created)?;
        }
    }

    let tsconfig = root.join("tsconfig.json");
    if tsconfig.exists() {
        kept.push(slash(&shown(&cwd, &tsconfig)));
    } else {
        write(tsconfig, TSCONFIG.as_bytes(), &mut created)?;
    }

    match crate::config::find(&cwd) {
        Some(config) => kept.push(slash(&shown(&cwd, &config))),
        None => {
            let key = serde_json::to_string(&slug)?.replace('"', "'");
            let path = serde_json::to_string(&slash(&shown(&root, &entry)))?.replace('"', "'");
            let text = format!(
                "// Значения по умолчанию для kine; флаги их перекрывают (kine docs config).\nimport {{ defineConfig }} from '@kine/core';\n\nexport default defineConfig({{\n  out: 'out',\n  scenes: {{ {key}: {path} }},\n}});\n"
            );
            write(root.join(crate::config::FILE), text.as_bytes(), &mut created)?;
        }
    }

    let gitignore = root.join(".gitignore");
    if gitignore.exists() {
        kept.push(slash(&shown(&cwd, &gitignore)));
    } else {
        write(gitignore, GITIGNORE.as_bytes(), &mut created)?;
    }

    Ok(Created {
        template: template_name.into(),
        dir: slash(&shown(&cwd, &dir)),
        root: slash(&shown(&cwd, &root)),
        component: (template_name != "blank").then_some(component),
        files: created,
        kept,
        entry: entry_shown,
    })
}

fn shown(cwd: &Path, p: &Path) -> PathBuf {
    match p.strip_prefix(cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => PathBuf::from("."),
        Ok(rel) => rel.to_path_buf(),
        Err(_) => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_names() {
        assert_eq!(component_name("lower-third", "X"), "LowerThird");
        assert_eq!(component_name("scenes/score_board2", "X"), "ScoreBoard2");
        assert_eq!(component_name("плашка", "LowerThird"), "LowerThird");
        assert_eq!(component_name("2nd", "LowerThird"), "LowerThird");
    }

    #[test]
    fn relative_import_paths() {
        assert_eq!(relative(Path::new("/p/scenes/lower"), Path::new("/p/fonts/inter")), "../../fonts/inter");
        assert_eq!(relative(Path::new("/p"), Path::new("/p/fonts/inter")), "./fonts/inter");
    }
}
