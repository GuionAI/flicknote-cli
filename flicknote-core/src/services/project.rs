//! Project application service.

use crate::backend::NoteDb;
use crate::error::CliError;
use crate::types::Project;

use super::dto::{
    Patch, ProjectAddInput, ProjectDto, ProjectModifyInput, ShareResult, UnshareResult,
};
use super::error::ServiceError;
use super::ports::{ShareGateway, ShareResource};

pub struct ProjectService<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> ProjectService<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }

    pub async fn list(&self, include_archived: bool) -> Result<Vec<ProjectDto>, ServiceError> {
        let mut projects = self.db.list_projects(false).await?;
        if include_archived {
            projects.extend(self.db.list_projects(true).await?);
            projects.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        }
        Ok(projects.into_iter().map(ProjectDto::from).collect())
    }

    pub async fn get(&self, project_id: &str) -> Result<ProjectDto, ServiceError> {
        let id = self.resolve_project_id(project_id).await?;
        Ok(self.db.find_project(&id).await?.into())
    }

    pub async fn add(&self, input: ProjectAddInput) -> Result<ProjectDto, ServiceError> {
        if input.name.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "project name must not be empty".to_string(),
            ));
        }
        if self.db.find_project_by_name(&input.name).await?.is_some() {
            return Err(ServiceError::InvalidArgument(format!(
                "project {:?} already exists",
                input.name
            )));
        }
        let id = self.db.create_project(&input.name).await?;
        if input.color.is_some() {
            self.db
                .update_project(&id, input.color.as_deref().map(Some))
                .await?;
        }
        Ok(self.db.find_project(&id).await?.into())
    }

    pub async fn modify(&self, input: ProjectModifyInput) -> Result<ProjectDto, ServiceError> {
        if input.color.is_missing() {
            return Err(ServiceError::NothingToModify);
        }
        let id = self.resolve_project_id(&input.id).await?;
        let color = match input.color {
            Patch::Missing => None,
            Patch::Null => Some(None),
            Patch::Value(color) => Some(Some(color)),
        };
        self.db
            .update_project(&id, color.as_ref().map(|value| value.as_deref()))
            .await?;
        Ok(self.db.find_project(&id).await?.into())
    }

    pub async fn archive(&self, project_id: &str) -> Result<ProjectDto, ServiceError> {
        let id = self.resolve_project_id(project_id).await?;
        self.db.delete_project(&id).await?;
        Ok(self.db.find_project(&id).await?.into())
    }

    pub async fn share(
        &self,
        gateway: &dyn ShareGateway,
        project_id: &str,
    ) -> Result<ShareResult, ServiceError> {
        let id = self.resolve_project_id(project_id).await?;
        let url = gateway.share(ShareResource::Project, &id).await?;
        Ok(ShareResult { url })
    }

    pub async fn unshare(
        &self,
        gateway: &dyn ShareGateway,
        project_id: &str,
    ) -> Result<UnshareResult, ServiceError> {
        let id = self.resolve_project_id(project_id).await?;
        gateway.unshare(ShareResource::Project, &id).await?;
        Ok(UnshareResult { revoked: true })
    }

    async fn resolve_project_id(&self, input: &str) -> Result<String, ServiceError> {
        self.db
            .resolve_project_id(input)
            .await
            .map_err(|error| match error {
                CliError::Other(_) => ServiceError::ProjectNotFound(input.to_string()),
                other => ServiceError::from(other),
            })
    }
}

impl From<Project> for ProjectDto {
    fn from(project: Project) -> Self {
        Self {
            id: project.id,
            name: project.name,
            color: project.color,
            archived: project.is_archived.unwrap_or(0) != 0,
            created_at: project.created_at,
        }
    }
}

#[cfg(all(test, feature = "powersync"))]
mod tests {

    use crate::backend::NoteDb;
    use crate::services::dto::{Patch, ProjectAddInput, ProjectModifyInput};
    use crate::services::ports::{ShareGateway, ShareResource};
    use crate::services::test_support::make_backend;
    use async_trait::async_trait;

    use super::ProjectService;

    #[tokio::test]
    async fn add_get_modify_and_archive_share_one_typed_contract() {
        let backend = make_backend().await;
        let service = ProjectService::new(&*backend);

        let created = service
            .add(ProjectAddInput {
                name: "work".to_string(),
                color: Some("#123456".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(created.name, "work");
        let duplicate = service
            .add(ProjectAddInput {
                name: "work".to_string(),
                color: None,
            })
            .await
            .unwrap_err();
        assert_eq!(duplicate.code(), "invalid_argument");

        let modified = service
            .modify(ProjectModifyInput {
                id: created.id.clone(),
                color: Patch::Value("#abcdef".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(modified.color.as_deref(), Some("#abcdef"));

        let archived = service.archive(&created.id).await.unwrap();
        assert!(archived.archived);
        assert!(service.list(false).await.unwrap().is_empty());
        assert_eq!(service.list(true).await.unwrap().len(), 1);
    }

    #[derive(Default)]
    struct FakeGateway(std::sync::Mutex<Vec<(ShareResource, String)>>);

    #[async_trait]
    impl ShareGateway for FakeGateway {
        async fn share(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<String, crate::services::error::ServiceError> {
            self.0.lock().unwrap().push((resource, id.to_string()));
            Ok("https://share.example/project".to_string())
        }

        async fn unshare(
            &self,
            resource: ShareResource,
            id: &str,
        ) -> Result<(), crate::services::error::ServiceError> {
            self.0.lock().unwrap().push((resource, id.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn share_and_unshare_resolve_project_before_gateway_call() {
        let backend = make_backend().await;
        let id = backend.create_project("work").await.unwrap();
        let gateway = FakeGateway::default();
        let service = ProjectService::new(&*backend);

        assert_eq!(
            service.share(&gateway, &id).await.unwrap().url,
            "https://share.example/project"
        );
        assert!(service.unshare(&gateway, &id).await.unwrap().revoked);
        assert_eq!(
            gateway.0.lock().unwrap().as_slice(),
            &[
                (ShareResource::Project, id.clone()),
                (ShareResource::Project, id)
            ]
        );
    }

    #[tokio::test]
    async fn project_lookup_uses_domain_error_code() {
        let backend = make_backend().await;
        let service = ProjectService::new(&*backend);

        let missing = service
            .get("550e8400-e29b-41d4-a716-446655440000")
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "project_not_found");
    }
}
