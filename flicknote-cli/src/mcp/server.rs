use std::sync::Arc;

use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{
    NoteAddInput, NoteArchiveResult, NoteCountInput, NoteDetail, NoteFindInput, NoteListInput,
    NoteModifyInput, NoteMutationResult, NoteSectionResult, NoteSummary, OpenResult,
    ProjectAddInput, ProjectDto, ProjectModifyInput, ShareResult, UnshareResult,
};
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::BrowserOpener;
use flicknote_core::services::source::SourceResult;
use flicknote_sync::ipc::{AppRequest, AppResult, DaemonClient};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::JsonSchema;
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Serialize;

use super::dto::{
    McpNoteArchiveResult, McpNoteDetail, McpNoteListResult, McpNoteMutationResult, McpNoteSummary,
    McpProjectDto, McpProjectListResult, McpSourceResult,
};
use super::error::tool_error;
use super::note_tools::*;
use super::project_tools::*;
use crate::commands::open::SystemBrowserOpener;

#[cfg(test)]
pub(crate) const EXPECTED_TOOLS: [&str; 25] = [
    "note_add",
    "note_append",
    "note_archive",
    "note_count",
    "note_delete_section",
    "note_find",
    "note_get",
    "note_get_section",
    "note_insert",
    "note_list",
    "note_modify",
    "note_open",
    "note_rename_section",
    "note_replace_section",
    "note_restore",
    "note_share",
    "note_source",
    "note_unshare",
    "project_add",
    "project_archive",
    "project_get",
    "project_list",
    "project_modify",
    "project_share",
    "project_unshare",
];

#[derive(Debug, Serialize, JsonSchema)]
struct CountResult {
    count: u64,
}

#[derive(Clone)]
pub(crate) struct FlickNoteMcp {
    config: Arc<Config>,
    tool_router: ToolRouter<Self>,
}

impl FlickNoteMcp {
    pub(crate) fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            tool_router: Self::tool_router(),
        }
    }

    async fn call<T: AppResult>(&self, request: AppRequest) -> Result<T, ServiceError> {
        DaemonClient::new(&self.config).call(request).await
    }

    fn effective_project(project: Option<String>) -> Option<String> {
        Self::select_project(project, std::env::var("FLICKNOTE_PROJECT").ok())
    }

    fn select_project(explicit: Option<String>, inherited: Option<String>) -> Option<String> {
        explicit.or_else(|| inherited.filter(|value| !value.is_empty()))
    }

    async fn resolve_project_name(&self, name: &str) -> Result<String, ServiceError> {
        self.call::<ProjectDto>(AppRequest::ProjectGetByName {
            name: name.to_string(),
        })
        .await
        .map(|project| project.id)
    }
}

fn structured<T>(result: Result<T, ServiceError>) -> Result<Json<T>, CallToolResult> {
    result.map(Json).map_err(|error| tool_error(&error))
}

#[tool_router(router = tool_router)]
impl FlickNoteMcp {
    #[tool(
        name = "note_list",
        description = "List active or archived notes with optional type and project filters.",
        annotations(read_only_hint = true)
    )]
    async fn note_list(
        &self,
        Parameters(params): Parameters<NoteListParams>,
    ) -> Result<Json<McpNoteListResult>, CallToolResult> {
        structured(
            self.call::<Vec<NoteSummary>>(AppRequest::NoteList(NoteListInput {
                note_type: params.note_type.map(|value| value.as_str().to_string()),
                project: Self::effective_project(params.project),
                archived: params.archived,
                limit: params.limit,
            }))
            .await
            .map(|notes| McpNoteListResult {
                notes: notes.into_iter().map(Into::into).collect(),
            }),
        )
    }

    #[tool(
        name = "note_find",
        description = "Find notes by OR keywords and exact extraction filters.",
        annotations(read_only_hint = true)
    )]
    async fn note_find(
        &self,
        Parameters(params): Parameters<NoteFindParams>,
    ) -> Result<Json<McpNoteListResult>, CallToolResult> {
        structured(
            self.call::<Vec<NoteSummary>>(AppRequest::NoteFind(NoteFindInput {
                keywords: params.keywords,
                extractions: params.extractions,
                project: Self::effective_project(params.project),
                archived: params.archived,
                limit: params.limit,
            }))
            .await
            .map(|notes| McpNoteListResult {
                notes: notes.into_iter().map(Into::into).collect(),
            }),
        )
    }

    #[tool(
        name = "note_count",
        description = "Count active or archived notes with optional OR keywords, project, and type.",
        annotations(read_only_hint = true)
    )]
    async fn note_count(
        &self,
        Parameters(params): Parameters<NoteCountParams>,
    ) -> Result<Json<CountResult>, CallToolResult> {
        structured(
            self.call::<u64>(AppRequest::NoteCount(NoteCountInput {
                keywords: params.keywords,
                project: Self::effective_project(params.project),
                note_type: params.note_type.map(|value| value.as_str().to_string()),
                archived: params.archived,
            }))
            .await
            .map(|count| CountResult { count }),
        )
    }

    #[tool(
        name = "note_get",
        description = "Get one note with editable content, metadata, extractions, and section tree.",
        annotations(read_only_hint = true)
    )]
    async fn note_get(
        &self,
        Parameters(params): Parameters<NoteGetParams>,
    ) -> Result<Json<McpNoteDetail>, CallToolResult> {
        structured(
            self.call::<NoteDetail>(AppRequest::NoteGet {
                id: params.id.to_string(),
                archived: params.archived,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_get_section",
        description = "Get a complete active-note section including its heading and child subsections.",
        annotations(read_only_hint = true)
    )]
    async fn note_get_section(
        &self,
        Parameters(params): Parameters<NoteSectionParams>,
    ) -> Result<Json<NoteSectionResult>, CallToolResult> {
        structured(
            self.call(AppRequest::NoteGetSection {
                id: params.id.to_string(),
                section: params.section,
            })
            .await,
        )
    }

    #[tool(
        name = "note_source",
        description = "Read stored source data as rendered content, raw JSON/text, or compact info. Normal notes often have no source data; use note_get for editable content. Use info then a 1-based range for large text or meeting sources.",
        annotations(read_only_hint = true)
    )]
    async fn note_source(
        &self,
        Parameters(params): Parameters<NoteSourceParams>,
    ) -> Result<Json<McpSourceResult>, CallToolResult> {
        structured(
            self.call::<SourceResult>(AppRequest::NoteSource {
                id: params.id.to_string(),
                archived: params.archived,
                view: params.view,
                range: params.range,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_add",
        description = "Create a note through the sync daemon. A leading H1 becomes the title; a pure HTTP(S) value becomes a link note.",
        annotations(open_world_hint = true)
    )]
    async fn note_add(
        &self,
        Parameters(params): Parameters<NoteAddParams>,
    ) -> Result<Json<McpNoteSummary>, CallToolResult> {
        structured(
            self.call::<NoteSummary>(AppRequest::NoteAdd(NoteAddInput {
                content: params.content,
                project: Self::effective_project(params.project),
                interpret_as_url: true,
                topics: Vec::new(),
                created_at: None,
            }))
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_modify",
        description = "Apply one exact before/after edit and/or change project or flagged state. Before and after are direct JSON fields."
    )]
    async fn note_modify(
        &self,
        Parameters(params): Parameters<NoteModifyParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteModify(NoteModifyInput {
                id: params.id.to_string(),
                before: params.before,
                after: params.after,
                section: params.section,
                project: params.project,
                flagged: params.flagged,
            }))
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_append",
        description = "Append text to an active note without requeueing AI processing."
    )]
    async fn note_append(
        &self,
        Parameters(params): Parameters<NoteContentParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteAppend {
                id: params.id.to_string(),
                content: params.content,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_insert",
        description = "Insert content before or after a complete section subtree."
    )]
    async fn note_insert(
        &self,
        Parameters(params): Parameters<NoteInsertParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteInsert {
                id: params.id.to_string(),
                section: params.section,
                position: params.position,
                content: params.content,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_replace_section",
        description = "Replace a complete section subtree. Content must begin with a Markdown heading."
    )]
    async fn note_replace_section(
        &self,
        Parameters(params): Parameters<NoteSectionContentParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteReplaceSection {
                id: params.id.to_string(),
                section: params.section,
                content: params.content,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_rename_section",
        description = "Rename a section heading while preserving its level."
    )]
    async fn note_rename_section(
        &self,
        Parameters(params): Parameters<NoteRenameSectionParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteRenameSection {
                id: params.id.to_string(),
                section: params.section,
                name: params.name,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_delete_section",
        description = "Delete a complete section subtree from an active note.",
        annotations(destructive_hint = true)
    )]
    async fn note_delete_section(
        &self,
        Parameters(params): Parameters<NoteSectionParams>,
    ) -> Result<Json<McpNoteMutationResult>, CallToolResult> {
        structured(
            self.call::<NoteMutationResult>(AppRequest::NoteDeleteSection {
                id: params.id.to_string(),
                section: params.section,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_archive",
        description = "Archive an active note using a soft delete.",
        annotations(destructive_hint = true)
    )]
    async fn note_archive(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<Json<McpNoteArchiveResult>, CallToolResult> {
        structured(
            self.call::<NoteArchiveResult>(AppRequest::NoteArchive {
                id: params.id.to_string(),
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_restore",
        description = "Restore one explicitly identified archived note."
    )]
    async fn note_restore(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<Json<McpNoteArchiveResult>, CallToolResult> {
        structured(
            self.call::<NoteArchiveResult>(AppRequest::NoteRestore {
                id: params.id.to_string(),
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "note_share",
        description = "Get or create a note share URL through the sync daemon.",
        annotations(open_world_hint = true)
    )]
    async fn note_share(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<Json<ShareResult>, CallToolResult> {
        structured(
            self.call(AppRequest::NoteShare {
                id: params.id.to_string(),
            })
            .await,
        )
    }

    #[tool(
        name = "note_unshare",
        description = "Revoke a note share URL through the sync daemon.",
        annotations(open_world_hint = true)
    )]
    async fn note_unshare(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<Json<UnshareResult>, CallToolResult> {
        structured(
            self.call(AppRequest::NoteUnshare {
                id: params.id.to_string(),
            })
            .await,
        )
    }

    #[tool(
        name = "note_open",
        description = "Open a note in the default browser and return the URL. This has a desktop side effect.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn note_open(
        &self,
        Parameters(params): Parameters<NoteIdParams>,
    ) -> Result<Json<OpenResult>, CallToolResult> {
        let mut result: OpenResult = self
            .call(AppRequest::NoteOpen {
                id: params.id.to_string(),
            })
            .await
            .map_err(|error| tool_error(&error))?;
        SystemBrowserOpener
            .open(&result.url)
            .map_err(|error| tool_error(&error))?;
        result.opened = true;
        Ok(Json(result))
    }

    #[tool(
        name = "project_list",
        description = "List active projects, optionally including archived projects.",
        annotations(read_only_hint = true)
    )]
    async fn project_list(
        &self,
        Parameters(params): Parameters<ProjectListParams>,
    ) -> Result<Json<McpProjectListResult>, CallToolResult> {
        structured(
            self.call::<Vec<ProjectDto>>(AppRequest::ProjectList {
                include_archived: params.include_archived,
            })
            .await
            .map(|projects| McpProjectListResult {
                projects: projects.into_iter().map(Into::into).collect(),
            }),
        )
    }

    #[tool(
        name = "project_get",
        description = "Get one active project by name.",
        annotations(read_only_hint = true)
    )]
    async fn project_get(
        &self,
        Parameters(params): Parameters<ProjectIdParams>,
    ) -> Result<Json<McpProjectDto>, CallToolResult> {
        structured(
            self.call::<ProjectDto>(AppRequest::ProjectGetByName {
                name: params.project,
            })
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "project_add",
        description = "Create a project with an optional color."
    )]
    async fn project_add(
        &self,
        Parameters(params): Parameters<ProjectAddParams>,
    ) -> Result<Json<McpProjectDto>, CallToolResult> {
        structured(
            self.call::<ProjectDto>(AppRequest::ProjectAdd(ProjectAddInput {
                name: params.name,
                color: params.color,
            }))
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "project_modify",
        description = "Patch a project color by name. Missing leaves it unchanged, null clears it, and a string sets it."
    )]
    async fn project_modify(
        &self,
        Parameters(params): Parameters<ProjectModifyParams>,
    ) -> Result<Json<McpProjectDto>, CallToolResult> {
        let project_id = self
            .resolve_project_name(&params.project)
            .await
            .map_err(|error| tool_error(&error))?;
        structured(
            self.call::<ProjectDto>(AppRequest::ProjectModify(ProjectModifyInput {
                id: project_id,
                color: params.color,
            }))
            .await
            .map(Into::into),
        )
    }

    #[tool(
        name = "project_archive",
        description = "Archive a project.",
        annotations(destructive_hint = true)
    )]
    async fn project_archive(
        &self,
        Parameters(params): Parameters<ProjectIdParams>,
    ) -> Result<Json<McpProjectDto>, CallToolResult> {
        let project_id = self
            .resolve_project_name(&params.project)
            .await
            .map_err(|error| tool_error(&error))?;
        structured(
            self.call::<ProjectDto>(AppRequest::ProjectArchive { id: project_id })
                .await
                .map(Into::into),
        )
    }

    #[tool(
        name = "project_share",
        description = "Get or create a project share URL through the sync daemon.",
        annotations(open_world_hint = true)
    )]
    async fn project_share(
        &self,
        Parameters(params): Parameters<ProjectIdParams>,
    ) -> Result<Json<ShareResult>, CallToolResult> {
        let project_id = self
            .resolve_project_name(&params.project)
            .await
            .map_err(|error| tool_error(&error))?;
        structured(self.call(AppRequest::ProjectShare { id: project_id }).await)
    }

    #[tool(
        name = "project_unshare",
        description = "Revoke a project share URL through the sync daemon.",
        annotations(open_world_hint = true)
    )]
    async fn project_unshare(
        &self,
        Parameters(params): Parameters<ProjectIdParams>,
    ) -> Result<Json<UnshareResult>, CallToolResult> {
        let project_id = self
            .resolve_project_name(&params.project)
            .await
            .map_err(|error| tool_error(&error))?;
        structured(
            self.call(AppRequest::ProjectUnshare { id: project_id })
                .await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FlickNoteMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("flicknote", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Daemon-backed FlickNote note and project tools. Every data tool requires the running FlickNote daemon.",
            )
    }
}

pub(crate) async fn serve(config: Arc<Config>) -> Result<(), CliError> {
    FlickNoteMcp::new(config)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| CliError::Other(format!("failed to initialize MCP server: {error}")))?
        .waiting()
        .await
        .map(|_| ())
        .map_err(|error| CliError::Other(format!("MCP server failed: {error}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use flicknote_core::config::Config;

    use super::FlickNoteMcp;

    fn assert_send<T: Send>(_: T) {}
    fn assert_send_sync<T: Send + Sync>() {}

    #[allow(dead_code)]
    fn assert_serve_future_is_send(config: Arc<Config>) {
        assert_send(super::serve(config));
    }

    #[test]
    fn mcp_service_is_send_and_sync() {
        assert_send_sync::<FlickNoteMcp>();
    }

    #[test]
    fn explicit_project_wins_then_falls_back_to_non_empty_environment_value() {
        assert_eq!(
            FlickNoteMcp::select_project(Some("explicit".into()), Some("environment".into())),
            Some("explicit".into())
        );
        assert_eq!(
            FlickNoteMcp::select_project(None, Some("environment".into())),
            Some("environment".into())
        );
        assert_eq!(
            FlickNoteMcp::select_project(None, Some(String::new())),
            None
        );
    }
}
