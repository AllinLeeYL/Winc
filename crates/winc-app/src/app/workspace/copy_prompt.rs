//! 从文件树将选中的文本文件格式化为可粘贴的 Markdown 提示词。

use std::io::Read;
use std::path::{Path, PathBuf};
use winc_backend::fs;
use winc_i18n::t;

use super::{App, UserEvent};

const MAX_FILES: usize = 128;
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

impl App {
    /// 在后台读取所选文件的已保存内容；目录不展开，路径使用其所在工作区目录的相对路径。
    pub(in crate::app) fn copy_as_prompt(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if paths.len() > MAX_FILES {
            self.flash(t!("menu-copy-as-prompt-too-many", limit = MAX_FILES));
            return;
        }
        let names = paths
            .iter()
            .map(|path| self.relative_path(path))
            .collect::<Vec<_>>();
        let remote = paths.iter().any(|path| winc_backend::is_remote(path));
        let read = move || collect_prompt(paths.into_iter().zip(names));
        let Some(proxy) = self.link.proxy.clone() else {
            let (prompt, skipped) = winc_backend::remote::allow_blocking(read);
            self.prompt_copied(prompt, skipped);
            return;
        };
        let task = move || {
            let (prompt, skipped) = read();
            let _ = proxy.send_event(UserEvent::PromptCopied(prompt, skipped));
        };
        if remote {
            winc_editor::background::spawn_blocking(task);
        } else {
            winc_editor::background::spawn(task);
        }
    }

    /// 在界面线程写入剪贴板；没有可复制的内容时保留剪贴板原值。
    pub(in crate::app) fn prompt_copied(&mut self, prompt: String, skipped: usize) {
        if prompt.is_empty() {
            self.flash(t!("menu-copy-as-prompt-empty"));
        } else {
            self.set_clipboard(prompt, false);
            self.flash(if skipped == 0 {
                t!("menu-copied-as-prompt").to_owned()
            } else {
                t!("menu-copied-as-prompt-skipped", count = skipped)
            });
        }
    }
}

fn collect_prompt(files: impl IntoIterator<Item = (PathBuf, Option<String>)>) -> (String, usize) {
    let mut prompt = String::new();
    let mut skipped = 0;
    for (path, name) in files {
        let block = name.and_then(|name| {
            let text = read_text(&path, MAX_PROMPT_BYTES - prompt.len())?;
            let language = path
                .extension()
                .and_then(|ext| ext.to_str())
                .filter(|ext| ext.bytes().all(|b| b.is_ascii_alphanumeric()))
                .unwrap_or("");
            let name = if cfg!(windows) && !winc_backend::is_remote(&path) {
                name.replace('\\', "/")
            } else {
                name
            };
            Some(file_block(&name, language, &text))
        });
        match block {
            Some(block) if prompt.len() + block.len() <= MAX_PROMPT_BYTES => {
                prompt.push_str(&block)
            }
            _ => skipped += 1,
        }
    }
    (prompt, skipped)
}

fn read_text(path: &Path, limit: usize) -> Option<String> {
    // 拒绝符号链接和特殊文件；同时限制读取量，以防文件在读取前后增长。
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len > limit as u64 {
        return None;
    }
    let mut file = fs::open(path, 0).ok()?;
    if !file.metadata.is_file() || file.metadata.len > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.reader()
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > limit {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    Some(text)
}

fn file_block(name: &str, language: &str, text: &str) -> String {
    // 围栏比文件中的任一连续反引号更长，避免 Markdown 文档里的代码块提前闭合外层。
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(3.max(longest + 1));
    let name = name.replace('\r', "\\r").replace('\n', "\\n");
    let newline = if text.is_empty() || text.ends_with(['\r', '\n']) {
        ""
    } else {
        "\n"
    };
    format!("File: {name}\n{fence}{language}\n{text}{newline}{fence}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_fence_contains_markdown_with_nested_fences() {
        assert_eq!(
            file_block("notes\nexample.md", "md", "```rs\nfn main() {}\n```"),
            "File: notes\\nexample.md\n````md\n```rs\nfn main() {}\n```\n````\n\n"
        );
        assert_eq!(file_block("README", "", ""), "File: README\n```\n```\n\n");
    }

    #[test]
    fn prompt_read_skips_invalid_text_and_files_over_budget() {
        let path = std::env::temp_dir().join(format!("winc-prompt-read-{}", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(read_text(&path, 5).as_deref(), Some("hello"));
        assert_eq!(read_text(&path, 4), None);
        std::fs::write(&path, b"image\x00data").unwrap();
        assert_eq!(read_text(&path, 20), None);
        std::fs::write(&path, b"\xff").unwrap();
        assert_eq!(read_text(&path, 20), None);
        std::fs::remove_file(path).unwrap();
    }
}
