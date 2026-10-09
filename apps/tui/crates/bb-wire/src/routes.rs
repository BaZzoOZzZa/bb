use std::fmt;

pub const API_PREFIX: &str = "/api/v1";
pub const HEALTH_PATH: &str = "/health";
pub const REALTIME_WS_PATH: &str = "/ws";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    pub const fn is_mutation(self) -> bool {
        !matches!(self, Self::Get)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Route {
    pub method: Method,
    pub path: String,
}

impl Route {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
        }
    }

    pub fn api_path(&self) -> String {
        format!("{API_PREFIX}{}", self.path)
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.api_path())
    }
}

pub fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn encode_file_path(path: &str) -> String {
    path.trim_start_matches('/')
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

pub fn terminal_ws_path(terminal_id: &str) -> String {
    format!("/ws/terminals/{}", encode_segment(terminal_id))
}

fn http_get(path: String) -> Route {
    Route::new(Method::Get, path)
}

fn http_post(path: String) -> Route {
    Route::new(Method::Post, path)
}

fn http_put(path: String) -> Route {
    Route::new(Method::Put, path)
}

fn http_patch(path: String) -> Route {
    Route::new(Method::Patch, path)
}

fn http_delete(path: String) -> Route {
    Route::new(Method::Delete, path)
}

fn thread(id: &str, suffix: &str) -> String {
    format!("/threads/{}{suffix}", encode_segment(id))
}

fn project(id: &str, suffix: &str) -> String {
    format!("/projects/{}{suffix}", encode_segment(id))
}

fn environment(id: &str, suffix: &str) -> String {
    format!("/environments/{}{suffix}", encode_segment(id))
}

fn terminal(id: &str, suffix: &str) -> String {
    format!("/terminals/{}{suffix}", encode_segment(id))
}

fn plugin(id: &str, suffix: &str) -> String {
    format!("/plugins/{}{suffix}", encode_segment(id))
}

pub mod system {
    use super::*;

    pub fn version() -> Route {
        http_get("/system/version".into())
    }

    pub fn attention() -> Route {
        http_get("/system/attention".into())
    }

    pub fn config() -> Route {
        http_get("/system/config".into())
    }

    pub fn reload_config() -> Route {
        http_post("/system/config/reload".into())
    }

    pub fn execution_options() -> Route {
        http_get("/system/execution-options".into())
    }

    pub fn environment_providers() -> Route {
        http_get("/system/environment-providers".into())
    }

    pub fn machine_providers() -> Route {
        http_get("/system/machine-providers".into())
    }

    pub fn providers() -> Route {
        http_get("/system/providers".into())
    }

    pub fn provider_states() -> Route {
        http_get("/system/providers/state".into())
    }

    pub fn provider_catalog() -> Route {
        http_get("/system/providers/catalog".into())
    }

    pub fn set_provider_enabled(id: &str) -> Route {
        http_put(format!("/system/providers/{}/enabled", encode_segment(id)))
    }

    pub fn usage_limits() -> Route {
        http_get("/system/usage-limits".into())
    }

    pub fn update_general_settings() -> Route {
        http_put("/settings/general".into())
    }

    pub fn update_keyboard_settings() -> Route {
        http_put("/settings/keyboard".into())
    }
}

pub mod projects {
    use super::*;

    pub fn list() -> Route {
        http_get("/projects".into())
    }

    pub fn create() -> Route {
        http_post("/projects".into())
    }

    pub fn sidebar_bootstrap() -> Route {
        http_get("/sidebar-bootstrap".into())
    }

    pub fn get(id: &str) -> Route {
        http_get(project(id, ""))
    }

    pub fn update(id: &str) -> Route {
        http_patch(project(id, ""))
    }

    pub fn delete(id: &str) -> Route {
        http_delete(project(id, ""))
    }

    pub fn reorder(id: &str) -> Route {
        http_patch(project(id, "/order"))
    }

    pub fn default_execution_options(id: &str) -> Route {
        http_get(project(id, "/default-execution-options"))
    }

    pub fn prompt_history(id: &str) -> Route {
        http_get(project(id, "/prompt-history"))
    }

    pub fn files(id: &str) -> Route {
        http_get(project(id, "/files"))
    }

    pub fn file(id: &str, file_path: &str) -> Route {
        http_get(project(
            id,
            &format!("/files/{}", encode_file_path(file_path)),
        ))
    }

    pub fn paths(id: &str) -> Route {
        http_get(project(id, "/paths"))
    }

    pub fn commands(id: &str) -> Route {
        http_get(project(id, "/commands"))
    }

    pub fn branches(id: &str) -> Route {
        http_get(project(id, "/branches"))
    }

    pub fn branch_options(id: &str) -> Route {
        http_get(project(id, "/branch-options"))
    }

    pub fn upload_attachment(id: &str) -> Route {
        http_post(project(id, "/attachments"))
    }

    pub fn attachment_content(id: &str) -> Route {
        http_get(project(id, "/attachments/content"))
    }
}

pub mod thread_sections {
    use super::*;

    pub fn list() -> Route {
        http_get("/thread-sections".into())
    }

    pub fn create() -> Route {
        http_post("/thread-sections".into())
    }

    pub fn update() -> Route {
        http_patch("/thread-sections".into())
    }

    pub fn delete() -> Route {
        http_delete("/thread-sections".into())
    }
}

pub mod threads {
    use super::*;

    pub fn list() -> Route {
        http_get("/threads".into())
    }

    pub fn count() -> Route {
        http_get("/threads/count".into())
    }

    pub fn running() -> Route {
        http_get("/threads/running".into())
    }

    pub fn search() -> Route {
        http_get("/threads/search".into())
    }

    pub fn resolve_mentions() -> Route {
        http_post("/threads/resolve-mentions".into())
    }

    pub fn create() -> Route {
        http_post("/threads".into())
    }

    pub fn fork() -> Route {
        http_post("/threads/fork".into())
    }

    pub fn get(id: &str) -> Route {
        http_get(thread(id, ""))
    }

    pub fn update(id: &str) -> Route {
        http_patch(thread(id, ""))
    }

    pub fn delete(id: &str) -> Route {
        http_delete(thread(id, ""))
    }

    pub fn plugin_metadata(id: &str) -> Route {
        http_get(thread(id, "/plugin-metadata"))
    }

    pub fn update_plugin_metadata(id: &str) -> Route {
        http_patch(thread(id, "/plugin-metadata"))
    }

    pub fn child_summary(id: &str) -> Route {
        http_get(thread(id, "/child-summary"))
    }

    pub fn send(id: &str) -> Route {
        http_post(thread(id, "/send"))
    }

    pub fn edit_message(id: &str) -> Route {
        http_post(thread(id, "/edit-message"))
    }

    pub fn retry(id: &str) -> Route {
        http_post(thread(id, "/retry"))
    }

    pub fn stop(id: &str) -> Route {
        http_post(thread(id, "/stop"))
    }

    pub fn compact(id: &str) -> Route {
        http_post(thread(id, "/compact"))
    }

    pub fn clear_context(id: &str) -> Route {
        http_post(thread(id, "/context/clear"))
    }

    pub fn cancel_plan(id: &str) -> Route {
        http_post(thread(id, "/plan/cancel"))
    }

    pub fn clear_goal(id: &str) -> Route {
        http_post(thread(id, "/goal/clear"))
    }

    pub fn open(id: &str) -> Route {
        http_post(thread(id, "/open"))
    }

    pub fn pin(id: &str) -> Route {
        http_post(thread(id, "/pin"))
    }

    pub fn unpin(id: &str) -> Route {
        http_post(thread(id, "/unpin"))
    }

    pub fn pin_order(id: &str) -> Route {
        http_patch(thread(id, "/pin-order"))
    }

    pub fn archive_all(id: &str) -> Route {
        http_post(thread(id, "/archive-all"))
    }

    pub fn unarchive(id: &str) -> Route {
        http_post(thread(id, "/unarchive"))
    }

    pub fn restore_environment(id: &str) -> Route {
        http_post(thread(id, "/restore-environment"))
    }

    pub fn mark_read(id: &str) -> Route {
        http_post(thread(id, "/read"))
    }

    pub fn mark_unread(id: &str) -> Route {
        http_post(thread(id, "/unread"))
    }

    pub fn timeline(id: &str) -> Route {
        http_get(thread(id, "/timeline"))
    }

    pub fn timeline_turn_summary_details(id: &str) -> Route {
        http_get(thread(id, "/timeline/turn-summary-details"))
    }

    pub fn context(id: &str) -> Route {
        http_get(thread(id, "/context"))
    }

    pub fn output(id: &str) -> Route {
        http_get(thread(id, "/output"))
    }

    pub fn message(id: &str, seq: i64) -> Route {
        http_get(thread(id, &format!("/messages/{seq}")))
    }

    pub fn default_execution_options(id: &str) -> Route {
        http_get(thread(id, "/default-execution-options"))
    }

    pub fn prompt_history(id: &str) -> Route {
        http_get(thread(id, "/prompt-history"))
    }

    pub fn queued_messages(id: &str) -> Route {
        http_get(thread(id, "/queued-messages"))
    }

    pub fn create_queued_message(id: &str) -> Route {
        http_post(thread(id, "/queued-messages"))
    }

    pub fn set_queued_message_group_boundary(id: &str) -> Route {
        http_patch(thread(id, "/queued-messages/group-boundary"))
    }

    fn queued(id: &str, queued_message_id: &str, suffix: &str) -> String {
        thread(
            id,
            &format!(
                "/queued-messages/{}{suffix}",
                encode_segment(queued_message_id)
            ),
        )
    }

    pub fn update_queued_message(id: &str, queued_message_id: &str) -> Route {
        http_patch(queued(id, queued_message_id, ""))
    }

    pub fn delete_queued_message(id: &str, queued_message_id: &str) -> Route {
        http_delete(queued(id, queued_message_id, ""))
    }

    pub fn send_queued_message(id: &str, queued_message_id: &str) -> Route {
        http_post(queued(id, queued_message_id, "/send"))
    }

    pub fn hold_queued_message_for_edit(id: &str, queued_message_id: &str) -> Route {
        http_post(queued(id, queued_message_id, "/edit-hold"))
    }

    pub fn release_queued_message_edit_hold(id: &str, queued_message_id: &str) -> Route {
        http_delete(queued(id, queued_message_id, "/edit-hold"))
    }

    pub fn reorder_queued_message(id: &str, queued_message_id: &str) -> Route {
        http_patch(queued(id, queued_message_id, "/order"))
    }

    pub fn interactions(id: &str) -> Route {
        http_get(thread(id, "/interactions"))
    }

    fn interaction_path(id: &str, interaction_id: &str, suffix: &str) -> String {
        thread(
            id,
            &format!("/interactions/{}{suffix}", encode_segment(interaction_id)),
        )
    }

    pub fn interaction(id: &str, interaction_id: &str) -> Route {
        http_get(interaction_path(id, interaction_id, ""))
    }

    pub fn resolve_interaction(id: &str, interaction_id: &str) -> Route {
        http_post(interaction_path(id, interaction_id, "/resolve"))
    }

    pub fn respond_to_interaction(id: &str, interaction_id: &str) -> Route {
        http_post(interaction_path(id, interaction_id, "/respond"))
    }

    pub fn cancel_interaction(id: &str, interaction_id: &str) -> Route {
        http_post(interaction_path(id, interaction_id, "/cancel"))
    }

    pub fn storage_location(id: &str) -> Route {
        http_get(thread(id, "/thread-storage/location"))
    }

    pub fn storage_files(id: &str) -> Route {
        http_get(thread(id, "/thread-storage/files"))
    }

    pub fn storage_paths(id: &str) -> Route {
        http_get(thread(id, "/thread-storage/paths"))
    }

    pub fn storage_file(id: &str, file_path: &str) -> Route {
        http_get(thread(
            id,
            &format!("/thread-storage/files/{}", encode_file_path(file_path)),
        ))
    }
}

pub mod queue {
    use super::*;

    pub fn list() -> Route {
        http_get("/queued-messages".into())
    }
}

pub mod environments {
    use super::*;

    pub fn list() -> Route {
        http_get("/environments".into())
    }

    pub fn get(id: &str) -> Route {
        http_get(environment(id, ""))
    }

    pub fn update(id: &str) -> Route {
        http_patch(environment(id, ""))
    }

    pub fn delete(id: &str) -> Route {
        http_delete(environment(id, ""))
    }

    pub fn cleanup(id: &str) -> Route {
        http_post(environment(id, "/cleanup"))
    }

    pub fn status(id: &str) -> Route {
        http_get(environment(id, "/status"))
    }

    pub fn pull_request(id: &str) -> Route {
        http_get(environment(id, "/pull-request"))
    }

    pub fn diff(id: &str) -> Route {
        http_get(environment(id, "/diff"))
    }

    pub fn diff_files(id: &str) -> Route {
        http_get(environment(id, "/diff/files"))
    }

    pub fn diff_patch(id: &str) -> Route {
        http_post(environment(id, "/diff/patch"))
    }

    pub fn diff_file(id: &str) -> Route {
        http_get(environment(id, "/diff/file"))
    }

    pub fn diff_branches(id: &str) -> Route {
        http_get(environment(id, "/diff/branches"))
    }

    pub fn paths(id: &str) -> Route {
        http_get(environment(id, "/paths"))
    }

    pub fn file(id: &str, file_path: &str) -> Route {
        http_get(environment(
            id,
            &format!("/files/{}", encode_file_path(file_path)),
        ))
    }

    pub fn actions(id: &str) -> Route {
        http_post(environment(id, "/actions"))
    }

    pub fn archive_threads(id: &str) -> Route {
        http_post(environment(id, "/archive-threads"))
    }
}

pub mod hosts {
    use super::*;

    pub fn list() -> Route {
        http_get("/hosts".into())
    }

    pub fn get(id: &str) -> Route {
        http_get(format!("/hosts/{}", encode_segment(id)))
    }

    pub fn update(id: &str) -> Route {
        http_patch(format!("/hosts/{}", encode_segment(id)))
    }

    pub fn permission_ceiling(id: &str) -> Route {
        http_patch(format!("/hosts/{}/permission-ceiling", encode_segment(id)))
    }

    pub fn provider_cli_status(id: &str) -> Route {
        http_get(format!(
            "/hosts/{}/provider-clis/status",
            encode_segment(id)
        ))
    }
}

pub mod terminals {
    use super::*;

    pub fn list() -> Route {
        http_get("/terminals".into())
    }

    pub fn create() -> Route {
        http_post("/terminals".into())
    }

    pub fn get(id: &str) -> Route {
        http_get(terminal(id, ""))
    }

    pub fn update(id: &str) -> Route {
        http_patch(terminal(id, ""))
    }

    pub fn restart(id: &str) -> Route {
        http_post(terminal(id, "/restart"))
    }

    pub fn close(id: &str) -> Route {
        http_post(terminal(id, "/close"))
    }

    pub fn input(id: &str) -> Route {
        http_post(terminal(id, "/input"))
    }

    pub fn resize(id: &str) -> Route {
        http_post(terminal(id, "/resize"))
    }

    pub fn output(id: &str) -> Route {
        http_get(terminal(id, "/output"))
    }
}

pub mod plugins {
    use super::*;

    pub fn list() -> Route {
        http_get("/plugins".into())
    }

    pub fn contributions() -> Route {
        http_get("/plugins/contributions".into())
    }

    pub fn mention_search() -> Route {
        http_get("/plugins/mentions/search".into())
    }

    pub fn cli(id: &str) -> Route {
        http_post(plugin(id, "/cli"))
    }

    pub fn settings(id: &str) -> Route {
        http_get(plugin(id, "/settings"))
    }

    pub fn update_settings(id: &str) -> Route {
        http_put(plugin(id, "/settings"))
    }

    pub fn enable(id: &str) -> Route {
        http_post(plugin(id, "/enable"))
    }

    pub fn disable(id: &str) -> Route {
        http_post(plugin(id, "/disable"))
    }

    pub fn logs(id: &str) -> Route {
        http_get(plugin(id, "/logs"))
    }

    pub fn rpc(id: &str, method: &str) -> Route {
        http_post(plugin(id, &format!("/rpc/{}", encode_segment(method))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_api_paths_with_encoded_segments() {
        let route = threads::resolve_interaction("thr_1", "int/2");
        assert_eq!(route.method, Method::Post);
        assert_eq!(
            route.api_path(),
            "/api/v1/threads/thr_1/interactions/int%2F2/resolve"
        );
    }

    #[test]
    fn file_paths_keep_separators_and_encode_segments() {
        let route = environments::file("env_1", "/src/my file.rs");
        assert_eq!(route.path, "/environments/env_1/files/src/my%20file.rs");
    }

    #[test]
    fn plugin_rpc_method_is_encoded() {
        assert_eq!(
            plugins::rpc("automations", "automations_list").to_string(),
            "POST /api/v1/plugins/automations/rpc/automations_list"
        );
        assert_eq!(terminal_ws_path("term 1"), "/ws/terminals/term%201");
    }

    #[test]
    fn collection_routes_are_not_confused_with_item_routes() {
        let cases = [
            (projects::list(), "GET /api/v1/projects"),
            (projects::get("p"), "GET /api/v1/projects/p"),
            (projects::delete("p"), "DELETE /api/v1/projects/p"),
            (
                projects::sidebar_bootstrap(),
                "GET /api/v1/sidebar-bootstrap",
            ),
            (threads::list(), "GET /api/v1/threads"),
            (threads::get("t"), "GET /api/v1/threads/t"),
            (threads::delete("t"), "DELETE /api/v1/threads/t"),
            (threads::search(), "GET /api/v1/threads/search"),
            (threads::timeline("t"), "GET /api/v1/threads/t/timeline"),
            (
                threads::delete_queued_message("t", "q"),
                "DELETE /api/v1/threads/t/queued-messages/q",
            ),
            (thread_sections::delete(), "DELETE /api/v1/thread-sections"),
            (environments::list(), "GET /api/v1/environments"),
            (environments::get("e"), "GET /api/v1/environments/e"),
            (hosts::list(), "GET /api/v1/hosts"),
            (hosts::get("h"), "GET /api/v1/hosts/h"),
            (terminals::list(), "GET /api/v1/terminals"),
            (terminals::get("x"), "GET /api/v1/terminals/x"),
            (plugins::list(), "GET /api/v1/plugins"),
            (plugins::settings("p"), "GET /api/v1/plugins/p/settings"),
            (queue::list(), "GET /api/v1/queued-messages"),
            (system::version(), "GET /api/v1/system/version"),
            (
                system::update_keyboard_settings(),
                "PUT /api/v1/settings/keyboard",
            ),
        ];
        for (route, expected) in cases {
            assert_eq!(route.to_string(), expected);
        }
    }

    #[test]
    fn non_ascii_segments_are_percent_encoded_as_utf8() {
        assert_eq!(encode_segment("тест"), "%D1%82%D0%B5%D1%81%D1%82");
    }
}
