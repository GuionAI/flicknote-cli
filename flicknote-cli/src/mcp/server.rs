use std::cell::OnceCell;
use std::rc::Rc;

use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{
    NoteAddInput, NoteModifyInput, NoteSectionResult, OpenResult, Patch, ProjectAddInput,
    ProjectModifyInput, ShareResult, UnshareResult,
};
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::{NoteCountInput, NoteFindInput, NoteListInput, NoteService};
use flicknote_core::services::project::ProjectService;
use flicknote_core::services::source::SourceResult;
use flicknote_sync::ipc::DaemonClient;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::JsonSchema;
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Serialize;

use crate::commands::open::SystemBrowserOpener;
use crate::gateway::GatewayClient;

use super::dto::{
    McpNoteArchiveResult, McpNoteDetail, McpNoteMutationResult, McpNoteSummary, McpProjectDto,
};
use super::error::{
    gateway_config_error, gateway_invalid_response_error, gateway_tool_error, tool_error,
};
use super::gateway_tools::*;
use super::note_tools::*;
use super::project_tools::*;

#[cfg(test)]
pub(crate) const EXPECTED_TOOLS: [&str; 27] = [
    "gateway_web_fetch",
    "gateway_web_search",
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
    db: Rc<dyn NoteDb>,
    config: Rc<Config>,
    gateway_client: Rc<OnceCell<Result<GatewayClient, CliError>>>,
    tool_router: ToolRouter<Self>,
}

impl FlickNoteMcp {
    pub(crate) fn new(db: Rc<dyn NoteDb>, config: Rc<Config>) -> Self {
        Self {
            db,
            config,
            gateway_client: Rc::new(OnceCell::new()),
            tool_router: Self::tool_router(),
        }
    }

    fn note_service(&self) -> NoteService<'_> {
        NoteService::new(self.db.as_ref())
    }

    fn project_service(&self) -> ProjectService<'_> {
        ProjectService::new(self.db.as_ref())
    }

    fn gateway_client(&self) -> Result<&GatewayClient, CallToolResult> {
        self.gateway_client
            .get_or_init(|| GatewayClient::new(&self.config))
            .as_ref()
            .map_err(gateway_config_error)
    }

    fn effective_project(project: Option<String>) -> Option<String> {
        Self::select_project(project, std::env::var("FLICKNOTE_PROJECT").ok())
    }

    fn select_project(explicit: Option<String>, inherited: Option<String>) -> Option<String> {
        explicit.or_else(|| inherited.filter(|value| !value.is_empty()))
    }

    async fn resolve_project_name(&self, name: &str) -> Result<String, ServiceError> {
        self.db
            .find_project_by_name(name)
            .await?
            .ok_or_else(|| ServiceError::ProjectNotFound(name.to_string()))
    }
}

fn structured<T>(result: Result<T, ServiceError>) -> Result<Json<T>, CallToolResult> {
    result.map(Json).map_err(|error| tool_error(&error))
}

#[tool_router(router = tool_router)]
impl FlickNoteMcp {
    #[tool(
        name = "gateway_web_search",
        description = "Search the web through the configured FlickNote Gateway.",
        annotations(read_only_hint = true)
    )]
    async fn gateway_web_search(
        &self,
        Parameters(params): Parameters<GatewayWebSearchParams>,
    ) -> Result<Json<GatewayWebSearchResult>, CallToolResult> {
        let client = self.gateway_client()?;
        let response = client
            .request_json(
                reqwest::Method::POST,
                "/web/v1/search",
                &serde_json::json!({ "query": params.query }),
            )
            .await
            .map_err(|error| gateway_tool_error(&error))?;
        let response = response
            .json()
            .await
            .map_err(|_| gateway_invalid_response_error())?;
        Ok(Json(response))
    }

    #[tool(
        name = "gateway_web_fetch",
        description = "Fetch readable page content through the configured FlickNote Gateway.",
        annotations(read_only_hint = true)
    )]
    async fn gateway_web_fetch(
        &self,
        Parameters(params): Parameters<GatewayWebFetchParams>,
    ) -> Result<Json<GatewayWebFetchResult>, CallToolResult> {
        let client = self.gateway_client()?;
        let response = client
            .request_json(
                reqwest::Method::POST,
                "/web/v1/fetch",
                &serde_json::json!({ "url": params.url }),
            )
            .await
            .map_err(|error| gateway_tool_error(&error))?;
        let response = response
            .json()
            .await
            .map_err(|_| gateway_invalid_response_error())?;
        Ok(Json(response))
    }

    #[tool(
        name = "note_list",
        description = "List active or archived notes with optional type and project filters.",
        annotations(read_only_hint = true)
    )]
    async fn note_list(
        &self,
        Parameters(params): Parameters<NoteListParams>,
    ) -> Result<Json<Vec<McpNoteSummary>>, CallToolResult> {
        structured(
            self.note_service()
                .list(NoteListInput {
                    note_type: params.note_type.map(|value| value.as_str().to_string()),
                    project: Self::effective_project(params.project),
                    archived: params.archived,
                    limit: params.limit,
                })
                .await
                .map(|notes| notes.into_iter().map(Into::into).collect()),
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
    ) -> Result<Json<Vec<McpNoteSummary>>, CallToolResult> {
        structured(
            self.note_service()
                .find(NoteFindInput {
                    keywords: params.keywords,
                    extractions: params.extractions,
                    project: Self::effective_project(params.project),
                    archived: params.archived,
                    limit: params.limit,
                })
                .await
                .map(|notes| notes.into_iter().map(Into::into).collect()),
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
            self.note_service()
                .count(NoteCountInput {
                    keywords: params.keywords,
                    project: Self::effective_project(params.project),
                    note_type: params.note_type.map(|value| value.as_str().to_string()),
                    archived: params.archived,
                })
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
            self.note_service()
                .get(&params.id.to_string(), params.archived)
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
            self.note_service()
                .get_section(&params.id.to_string(), &params.section)
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
    ) -> Result<Json<SourceResult>, CallToolResult> {
        structured(
            self.note_service()
                .source(
                    &params.id.to_string(),
                    params.archived,
                    params.view,
                    params.range.as_deref(),
                )
                .await,
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
            self.note_service()
                .add(
                    &DaemonClient::new(&self.config),
                    NoteAddInput {
                        content: params.content,
                        project: Self::effective_project(params.project),
                        interpret_as_url: true,
                    },
                )
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
            self.note_service()
                .modify(NoteModifyInput {
                    id: params.id.to_string(),
                    before: params.before,
                    after: params.after,
                    section: params.section,
                    project: params.project,
                    flagged: params.flagged,
                })
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
            self.note_service()
                .append(&params.id.to_string(), &params.content)
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
            self.note_service()
                .insert(
                    &params.id.to_string(),
                    &params.section,
                    params.position,
                    &params.content,
                )
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
            self.note_service()
                .replace_section(&params.id.to_string(), &params.section, &params.content)
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
            self.note_service()
                .rename_section(&params.id.to_string(), &params.section, &params.name)
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
            self.note_service()
                .delete_section(&params.id.to_string(), &params.section)
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
            self.note_service()
                .archive(&params.id.to_string())
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
            self.note_service()
                .restore(&params.id.to_string())
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
            self.note_service()
                .share(&DaemonClient::new(&self.config), &params.id.to_string())
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
            self.note_service()
                .unshare(&DaemonClient::new(&self.config), &params.id.to_string())
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
        let Some(web_url) = self.config.web_url.as_deref() else {
            return Err(tool_error(&ServiceError::ConfigMissing(
                "webUrl".to_string(),
            )));
        };
        structured(
            self.note_service()
                .open(&SystemBrowserOpener, web_url, &params.id.to_string())
                .await,
        )
    }

    #[tool(
        name = "project_list",
        description = "List active projects, optionally including archived projects.",
        annotations(read_only_hint = true)
    )]
    async fn project_list(
        &self,
        Parameters(params): Parameters<ProjectListParams>,
    ) -> Result<Json<Vec<McpProjectDto>>, CallToolResult> {
        structured(
            self.project_service()
                .list(params.include_archived)
                .await
                .map(|projects| projects.into_iter().map(Into::into).collect()),
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
        let project_id = self
            .resolve_project_name(&params.project)
            .await
            .map_err(|error| tool_error(&error))?;
        structured(
            self.project_service()
                .get(&project_id)
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
            self.project_service()
                .add(ProjectAddInput {
                    name: params.name,
                    keyterm: None,
                    color: params.color,
                })
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
            self.project_service()
                .modify(ProjectModifyInput {
                    id: project_id,
                    keyterm: Patch::Missing,
                    color: params.color,
                })
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
            self.project_service()
                .archive(&project_id)
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
        structured(
            self.project_service()
                .share(&DaemonClient::new(&self.config), &project_id)
                .await,
        )
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
            self.project_service()
                .unshare(&DaemonClient::new(&self.config), &project_id)
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
                "Local-first FlickNote note and project tools. Network-backed add/share/unshare require the running sync daemon.",
            )
    }
}

pub(crate) async fn serve(db: Rc<dyn NoteDb>, config: Rc<Config>) -> Result<(), CliError> {
    FlickNoteMcp::new(db, config)
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
    use super::FlickNoteMcp;

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
