//! AI 提交信息总结：只发送准备提交的树差异，限制大批量改动和网络响应的大小。

use std::path::Path;
use std::process::Stdio;

use winc_config::Json;
use winc_editor::git::{Git, Partial};
use winc_i18n::{Locale, t};

/// 使用字节预算，对中文也能提供明确的请求大小上限。
const MAX_DIFF_BYTES: usize = 12_000;
const MAX_LINES_PER_FILE: usize = 150;

/// 连通性测试验证到了哪一层；显示时再翻译，支持测试后切换界面语言。
#[derive(Debug, Clone, Copy)]
pub(crate) enum ConnectionTest {
    /// 模型补全请求已成功。
    Completion,
    /// 模型列表已成功读取，还需指定模型来验证补全。
    Models,
}

/// 将基础 URL 规整为 `/chat/completions` 端点。
fn chat_completions_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

/// 将基础 URL 规整为 `/models` 端点。
fn models_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix("/chat/completions").unwrap_or(trimmed);
    if trimmed.ends_with("/models") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/models")
    }
}

/// 验证网络、HTTP 状态、API 错误和响应结构，失败不能显示成成功。
pub(crate) fn test_connection(
    endpoint: &str,
    key: Option<&str>,
    model: Option<&str>,
) -> Result<ConnectionTest, String> {
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        let response = request_completion(
            endpoint,
            key,
            model,
            vec![message("user", "Reply OK.")],
            2048,
            15,
        )?;
        extract_message(&response)?;
        Ok(ConnectionTest::Completion)
    } else {
        let response = request_json(&models_url(endpoint), key, None, 15)?;
        if response
            .as_object()
            .and_then(|o| o.get("data"))
            .and_then(Json::as_array)
            .is_none()
        {
            return Err(t!("settings-ai-invalid-response").to_owned());
        }
        Ok(ConnectionTest::Models)
    }
}

fn message(role: &str, text: &str) -> Json {
    Json::object([
        ("role", Json::String(role.to_owned())),
        ("content", Json::String(text.to_owned())),
    ])
}

/// 界面语言对应的内置提交说明规则，也作为设置编辑框的初始内容。
pub(crate) fn default_commit_prompt(locale: Locale) -> &'static str {
    match locale {
        Locale::ZhCn => {
            "你是一个专业的代码提交信息生成助手。请根据提供的代码改动（diff）生成简洁、高质量的 Git Commit Message。\n\
            要求：\n\
            1. 严格遵循 Conventional Commits 规范，例如 feat:、fix:、refactor:、docs:、style:、chore: 等。\n\
            2. 首行简明扼要概括主要改动（建议 50 字以内，不超过 72 字符）。\n\
            3. 若包含多项重点，可在空一行后使用无序列表（- ）补充列举重要细节。\n\
            4. 使用中文编写提交说明（前缀如 feat: 可保留英文）。\n\
            5. 直接输出最终的 Commit Message 本文，不要任何额外解释，不要用代码块包裹。"
        }
        Locale::EnUs => {
            "You are an expert developer assistant specialized in writing clean Git commit messages.\n\
            Analyze the provided diff and write a concise, high-quality commit message.\n\
            Requirements:\n\
            1. Follow the Conventional Commits specification (e.g. feat:, fix:, refactor:, docs:, chore:).\n\
            2. First line is a concise summary of the changes (<= 72 chars).\n\
            3. If there are multiple distinct changes, separate with a blank line and list bullet points.\n\
            4. Write the commit message in English.\n\
            5. Output ONLY the commit message text with no explanations and no markdown code fences."
        }
    }
}

/// 自定义规则替换内置规则；仅替换分支变量，不执行 shell 命令。
fn commit_prompt(locale: Locale, custom: Option<&str>, branch: Option<&str>) -> String {
    custom
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .unwrap_or_else(|| default_commit_prompt(locale))
        .replace("$GIT_BRANCH_NAME", branch.unwrap_or(""))
}

/// 按用户提示词（未配置时按界面语言的默认规则）总结所选差异。
pub(crate) fn generate_commit_message(
    endpoint: &str,
    key: Option<&str>,
    model: &str,
    diff_content: &str,
    locale: Locale,
    custom_prompt: Option<&str>,
    branch_name: Option<&str>,
) -> Result<String, String> {
    let system_prompt = commit_prompt(locale, custom_prompt, branch_name);

    let response = request_completion(
        endpoint,
        key,
        model,
        vec![
            message("system", &system_prompt),
            message("user", diff_content),
        ],
        4096,
        45,
    )?;
    extract_message(&response)
}

/// 使用实际提交共用的临时索引，包含重命名原路径和只勾选的行，不改真实暂存区。
pub(crate) fn prepare_commit_diff(
    git: &Git,
    root: &Path,
    paths: &[std::path::PathBuf],
    partial: &[Partial],
    amend: bool,
) -> Result<String, String> {
    let (stat, patch) = git.commit_diff(root, paths, partial, amend, 2 * 1024 * 1024)?;
    if stat.trim().is_empty() && patch.trim().is_empty() {
        return Err(t!("git-commit-nothing-checked").to_owned());
    }
    Ok(build_diff_prompt(&stat, &patch, MAX_DIFF_BYTES))
}

/// UTF-8 安全的字节前缀。
fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn append_bounded(out: &mut String, text: &str, limit: usize) {
    let room = limit.saturating_sub(out.len());
    if text.len() <= room {
        out.push_str(text);
    } else if room >= 5 {
        out.push_str(prefix(text, room - 5));
        out.push_str("\n...\n");
    } else {
        out.push_str(prefix(text, room));
    }
}

/// 概览最多用三分之一预算，为各源码文件分配片段空间，超长行同样受总预算约束。
fn build_diff_prompt(stat: &str, patch: &str, max_bytes: usize) -> String {
    let mut prompt = String::new();
    append_bounded(&mut prompt, "=== Summary of Changed Files ===\n", max_bytes);
    let summary_limit = (prompt.len() + max_bytes / 3).min(max_bytes);
    let stat = stat.trim();
    if let Some((files, total)) = stat.rsplit_once('\n') {
        append_bounded(&mut prompt, total, summary_limit);
        append_bounded(&mut prompt, "\n", summary_limit);
        append_bounded(&mut prompt, files, summary_limit);
    } else {
        append_bounded(&mut prompt, stat, summary_limit);
    }
    append_bounded(&mut prompt, "\n=== Key Changes Diff ===\n", max_bytes);
    let mut files = Vec::new();
    let starts: Vec<usize> = patch
        .match_indices("diff --git ")
        .filter(|(i, _)| *i == 0 || patch.as_bytes()[i - 1] == b'\n')
        .map(|(i, _)| i)
        .chain(std::iter::once(patch.len()))
        .collect();
    for pair in starts.windows(2) {
        files.push(&patch[pair[0]..pair[1]]);
    }
    let count = files.len();
    for (i, file) in files.into_iter().enumerate() {
        let budget = max_bytes.saturating_sub(prompt.len()) / (count - i).min(8);
        if budget == 0 {
            break;
        }
        let mut snippet = String::new();
        // 优先保留文件信息、hunk 标头和增删行，减少上下文行对预算的占用。
        for line in file
            .lines()
            .filter(|line| !line.starts_with(' '))
            .take(MAX_LINES_PER_FILE)
        {
            append_bounded(&mut snippet, line, budget);
            append_bounded(&mut snippet, "\n", budget);
            if snippet.len() >= budget.saturating_sub(4) {
                break;
            }
        }
        if snippet.len() < file.len() {
            let note = "\n[File diff abbreviated]\n";
            let keep = prefix(&snippet, budget.saturating_sub(note.len())).len();
            snippet.truncate(keep);
            append_bounded(&mut snippet, note, budget);
        }
        append_bounded(&mut prompt, &snippet, max_bytes);
    }
    if count == 0 {
        append_bounded(
            &mut prompt,
            "[No source text diff; see summary]\n",
            max_bytes,
        );
    }
    prompt
}

/// 不固定 temperature，使用新 token 参数；仅在兼容服务明确拒绝该参数时退回旧名称。
fn request_completion(
    endpoint: &str,
    key: Option<&str>,
    model: &str,
    messages: Vec<Json>,
    tokens: usize,
    timeout: u64,
) -> Result<Json, String> {
    let url = chat_completions_url(endpoint);
    let mut payload = Json::object([
        ("model", Json::String(model.to_owned())),
        ("messages", Json::Array(messages)),
        ("max_completion_tokens", Json::Number(tokens as f64)),
    ]);
    match request_json(&url, key, Some(&payload.write(0)), timeout) {
        Err(error)
            if error.contains("max_completion_tokens")
                && [
                    "unsupported",
                    "unknown",
                    "unrecognized",
                    "not supported",
                    "not allowed",
                ]
                .iter()
                .any(|s| error.to_ascii_lowercase().contains(s)) =>
        {
            if let Json::Object(fields) = &mut payload {
                fields.remove("max_completion_tokens");
                fields.insert("max_tokens".to_owned(), Json::Number(tokens as f64));
            }
            request_json(&url, key, Some(&payload.write(0)), timeout)
        }
        result => result,
    }
}

fn curl_quoted(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// 密钥和请求正文经 stdin 传给 curl，不放在可被进程列表读取的命令行中。
fn request_json(
    url: &str,
    key: Option<&str>,
    payload: Option<&str>,
    timeout: u64,
) -> Result<Json, String> {
    use std::io::Write;
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.chars().any(char::is_control)
    {
        return Err(t!("settings-ai-invalid-url").to_owned());
    }
    let mut config = format!(
        "url = \"{}\"\nheader = \"Accept: application/json\"\n",
        curl_quoted(url)
    );
    if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) {
        if key.chars().any(char::is_control) {
            return Err(t!("settings-ai-invalid-key").to_owned());
        }
        config.push_str(&format!(
            "header = \"Authorization: Bearer {}\"\n",
            curl_quoted(key)
        ));
    }
    if let Some(body) = payload {
        config.push_str(&format!(
            "header = \"Content-Type: application/json\"\ndata-binary = \"{}\"\n",
            curl_quoted(body)
        ));
    }
    let mut command = std::process::Command::new("curl");
    command
        .args([
            "-q",
            "--silent",
            "--show-error",
            "--globoff",
            "--proto",
            "=http,https",
            "--max-time",
            &timeout.to_string(),
            "--max-filesize",
            "1048576",
            "--write-out",
            "\n%{http_code}",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = command.spawn().map_err(|e| format!("curl: {e}"))?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "curl stdin unavailable".to_owned())
        .and_then(|mut stdin| {
            stdin
                .write_all(config.as_bytes())
                .map_err(|e| e.to_string())
        });
    let output = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    write_result?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout
        .rsplit_once('\n')
        .ok_or_else(|| t!("settings-ai-invalid-response").to_owned())?;
    let status = status.trim().parse::<u16>().unwrap_or(0);
    let json = Json::parse(body);
    if let Some(error) = json
        .as_ref()
        .and_then(Json::as_object)
        .and_then(|o| o.get("error"))
    {
        let error = error
            .as_object()
            .and_then(|o| o.get("message"))
            .and_then(Json::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("API error");
        return Err(format!("HTTP {status}: {error}"));
    }
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {}", prefix(body.trim(), 240)));
    }
    json.ok_or_else(|| t!("settings-ai-invalid-response").to_owned())
}

#[cfg(test)]
fn extract_message_from_response(response: &str) -> Result<String, String> {
    let json =
        Json::parse(response).ok_or_else(|| t!("settings-ai-invalid-response").to_owned())?;
    extract_message(&json)
}

fn extract_message(json: &Json) -> Result<String, String> {
    let choice = json
        .as_object()
        .and_then(|o| o.get("choices"))
        .and_then(Json::as_array)
        .and_then(|a| a.first())
        .and_then(Json::as_object)
        .ok_or_else(|| t!("settings-ai-invalid-response").to_owned())?;
    if choice.get("finish_reason").and_then(Json::as_str) == Some("length") {
        return Err(t!("git-commit-ai-truncated").to_owned());
    }
    let content = choice
        .get("message")
        .and_then(Json::as_object)
        .and_then(|o| o.get("content"))
        .and_then(Json::as_str)
        .ok_or_else(|| t!("settings-ai-invalid-response").to_owned())?;
    let message = clean_commit_message(content);
    if message.is_empty() {
        return Err(t!("git-commit-ai-empty").to_owned());
    }
    Ok(message)
}

/// 清洗 Commit Message（剥离 markdown 代码块、首尾引号和多余空白）。
fn clean_commit_message(raw: &str) -> String {
    let trimmed = raw.trim();
    let message = if let Some(stripped) = trimmed.strip_prefix("```") {
        let after_header = stripped.find('\n').map_or(stripped, |i| &stripped[i + 1..]);
        after_header
            .strip_suffix("```")
            .unwrap_or(after_header)
            .trim()
    } else {
        trimmed
    };
    let message = message
        .strip_prefix('"')
        .and_then(|m| m.strip_suffix('"'))
        .unwrap_or(message)
        .trim();
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalization_works() {
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            models_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn cleans_markdown_fences() {
        assert_eq!(
            clean_commit_message("```git\nfeat: add login\n```"),
            "feat: add login"
        );
        assert_eq!(
            clean_commit_message("\"feat: quoted message\""),
            "feat: quoted message"
        );
    }

    #[test]
    fn extracts_message_content() {
        let response = r#"{"choices":[{"message":{"content":"feat(ui): add button"}}]}"#;
        assert_eq!(
            extract_message_from_response(response).unwrap(),
            "feat(ui): add button"
        );
    }

    #[test]
    fn budgets_include_summary_headers_long_lines_and_unicode() {
        let stat = " 文件.rs | 100 +++++\n".repeat(2000);
        let patch = format!(
            "diff --git a/main.rs b/main.rs\n@@ -0,0 +1 @@\n+{}\ndiff --git a/other.rs b/other.rs\n+important\n",
            "中文".repeat(10000)
        );
        for limit in [0, 1, 30, 100, 1000, MAX_DIFF_BYTES] {
            let prompt = build_diff_prompt(&stat, &patch, limit);
            assert!(prompt.len() <= limit);
            if limit >= 1000 {
                assert!(prompt.contains("other.rs"));
                assert!(prompt.contains("important"));
            }
        }
    }

    #[test]
    fn invalid_unicode_and_empty_responses_are_errors() {
        assert!(extract_message_from_response(&"错误".repeat(100)).is_err());
        assert!(
            extract_message_from_response(r#"{"choices":[{"message":{"content":"  "}}]}"#).is_err()
        );
        assert!(
            extract_message_from_response(
                r#"{"choices":[{"finish_reason":"length","message":{"content":"feat: partial"}}]}"#
            )
            .is_err()
        );
    }

    fn mock_http(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                let mut size = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        size = value.trim().parse().unwrap();
                    }
                    request.push_str(&line);
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut data = vec![0; size];
                reader.read_exact(&mut data).unwrap();
                request.push_str(std::str::from_utf8(&data).unwrap());
                requests.push(request);
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (url, thread)
    }

    #[test]
    fn connection_test_rejects_http_api_and_invalid_body_errors() {
        for (status, body) in [
            (401, r#"{"error":{"message":"invalid API key"}}"#),
            (503, r#"{"data":[]}"#),
            (200, "网页错误页面"),
            (200, r#"{"error":{"message":"model not found"}}"#),
        ] {
            let (url, server) = mock_http(vec![(status, body.to_owned())]);
            assert!(test_connection(&url, None, None).is_err());
            server.join().unwrap();
        }
        let (url, server) = mock_http(vec![(200, r#"{"data":[]}"#.to_owned())]);
        assert!(matches!(
            test_connection(&url, None, None),
            Ok(ConnectionTest::Models)
        ));
        server.join().unwrap();
        let (url, server) = mock_http(vec![(200, r#"{"choices":[{}]}"#.to_owned())]);
        assert!(test_connection(&url, None, Some("model")).is_err());
        server.join().unwrap();
        let (url, server) = mock_http(vec![(
            200,
            r#"{"choices":[{"message":{"content":"OK"}}]}"#.to_owned(),
        )]);
        assert!(matches!(
            test_connection(&url, None, Some("model")),
            Ok(ConnectionTest::Completion)
        ));
        server.join().unwrap();
        assert!(test_connection("not-a-url", None, None).is_err());
    }

    #[test]
    fn completion_uses_compatible_parameters_and_preserves_unicode_payload() {
        let (url, server) = mock_http(vec![
            (400, r#"{"error":{"message":"Unknown parameter: max_completion_tokens"}}"#.to_owned()),
            (200, r#"{"choices":[{"finish_reason":"stop","message":{"content":"feat: 支持 AI 总结"}}]}"#.to_owned()),
        ]);
        let diff = "diff 中文 \"quoted\" \\ path\n+hello";
        let generated = generate_commit_message(
            &url,
            Some("sk-local-test"),
            "custom-model",
            diff,
            Locale::ZhCn,
            None,
            None,
        )
        .unwrap();
        assert_eq!(generated, "feat: 支持 AI 总结");
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        for (i, request) in requests.iter().enumerate() {
            assert!(request.starts_with("POST /v1/chat/completions "));
            assert!(request.contains("Authorization: Bearer sk-local-test"));
            let json = Json::parse(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let fields = json.as_object().unwrap();
            assert!(!fields.contains_key("temperature"));
            assert!(fields.contains_key(if i == 0 {
                "max_completion_tokens"
            } else {
                "max_tokens"
            }));
            let messages = fields["messages"].as_array().unwrap();
            assert_eq!(
                messages[1].as_object().unwrap()["content"].as_str(),
                Some(diff)
            );
        }
    }

    #[test]
    fn custom_commit_prompt_replaces_defaults_and_expands_branch_in_request() {
        let (url, server) = mock_http(vec![(
            200,
            r#"{"choices":[{"finish_reason":"stop","message":{"content":"ABC-123: Änderung"}}]}"#
                .to_owned(),
        )]);
        let custom =
            "Answer in German\nExtract the ticket from $GIT_BRANCH_NAME\nUse bullet points";
        let result = generate_commit_message(
            &url,
            None,
            "model",
            "+Änderung",
            Locale::ZhCn,
            Some(custom),
            Some("feat/ABC-123"),
        )
        .unwrap();
        assert_eq!(result, "ABC-123: Änderung");
        let requests = server.join().unwrap();
        let json = Json::parse(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        let messages = json.as_object().unwrap()["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].as_object().unwrap()["content"].as_str(),
            Some("Answer in German\nExtract the ticket from feat/ABC-123\nUse bullet points")
        );
        assert_eq!(
            messages[1].as_object().unwrap()["content"].as_str(),
            Some("+Änderung")
        );
    }

    #[test]
    fn blank_commit_prompt_uses_locale_and_detached_head_has_no_branch_name() {
        for locale in [Locale::ZhCn, Locale::EnUs] {
            for prompt in [None, Some(""), Some(" \n\t ")] {
                assert_eq!(
                    commit_prompt(locale, prompt, Some("main")),
                    default_commit_prompt(locale)
                );
            }
        }
        assert_eq!(
            commit_prompt(Locale::EnUs, Some("Branch: $GIT_BRANCH_NAME"), None),
            "Branch: "
        );
        // 分支名只作文字替换，保留 Unicode 和 shell 字符，也不递归展开。
        let branch = "修复/ABC-123-$(echo-x)-$GIT_BRANCH_NAME";
        assert_eq!(
            commit_prompt(Locale::EnUs, Some("$GIT_BRANCH_NAME"), Some(branch)),
            branch
        );
    }
}
