use crate::backend::{InsertCommentReq, NoteDb, UpdateCommentReq};
use crate::types::NoteComment;

use super::dto::{CommentBatchModifyInput, CommentCreateInput, CommentDto, CommentModifyInput};
use super::error::ServiceError;

pub struct CommentService<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> CommentService<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }

    pub async fn list(&self, note_id: &str) -> Result<Vec<CommentDto>, ServiceError> {
        let note_id = self.db.resolve_note_id(note_id).await?;
        self.db
            .list_comments(&note_id)
            .await?
            .into_iter()
            .map(comment_dto)
            .collect()
    }

    pub async fn create(&self, input: CommentCreateInput) -> Result<CommentDto, ServiceError> {
        if input.block_text.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "block_text must not be empty".to_string(),
            ));
        }
        if input.author.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "author must not be empty".to_string(),
            ));
        }
        let note_id = self.db.resolve_note_id(&input.note_id).await?;
        if let Some(parent_id) = input.parent_id.as_deref() {
            let parent = self
                .db
                .find_comment(parent_id)
                .await
                .map_err(|_| ServiceError::CommentNotFound(parent_id.to_string()))?;
            if parent.note_id != note_id {
                return Err(ServiceError::InvalidArgument(
                    "parent comment belongs to another note".to_string(),
                ));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let content = serde_json::to_string(&input.content)
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        self.db
            .insert_comment(&InsertCommentReq {
                id: &id,
                note_id: &note_id,
                block_text: &input.block_text,
                content: &content,
                author: &input.author,
                is_read: input.is_read,
                parent_id: input.parent_id.as_deref(),
                now: &now,
            })
            .await?;
        comment_dto(self.db.find_comment(&id).await?)
    }

    pub async fn modify(&self, input: CommentModifyInput) -> Result<CommentDto, ServiceError> {
        if input.content.is_none() && input.is_read.is_none() {
            return Err(ServiceError::NothingToModify);
        }
        self.db
            .find_comment(&input.id)
            .await
            .map_err(|_| ServiceError::CommentNotFound(input.id.clone()))?;
        let content = input
            .content
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        self.db
            .update_comment(&input.id, content.as_deref(), input.is_read)
            .await?;
        comment_dto(self.db.find_comment(&input.id).await?)
    }

    pub async fn modify_batch(
        &self,
        input: CommentBatchModifyInput,
    ) -> Result<Vec<CommentDto>, ServiceError> {
        if input.comments.is_empty() {
            return Err(ServiceError::InvalidArgument(
                "comment batch must not be empty".to_string(),
            ));
        }
        if input
            .comments
            .iter()
            .any(|comment| comment.content.is_none() && comment.is_read.is_none())
        {
            return Err(ServiceError::NothingToModify);
        }
        let updates = input
            .comments
            .iter()
            .map(|comment| {
                Ok(UpdateCommentReq {
                    id: comment.id.clone(),
                    content: comment
                        .content
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|error| ServiceError::Internal(error.to_string()))?,
                    is_read: comment.is_read,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        self.db.update_comments(&updates).await?;
        let mut comments = Vec::with_capacity(updates.len());
        for update in updates {
            comments.push(comment_dto(self.db.find_comment(&update.id).await?)?);
        }
        Ok(comments)
    }
}

fn comment_dto(comment: NoteComment) -> Result<CommentDto, ServiceError> {
    Ok(CommentDto {
        id: comment.id,
        note_id: comment.note_id,
        block_text: comment.block_text,
        content: serde_json::from_str(&comment.content)
            .map_err(|error| ServiceError::Internal(error.to_string()))?,
        author: comment.author,
        is_read: comment.is_read.unwrap_or(0) != 0,
        created_at: comment.created_at,
        parent_id: comment.parent_id,
    })
}

#[cfg(all(test, feature = "powersync"))]
mod tests {
    use crate::services::dto::{CommentBatchModifyInput, CommentCreateInput, CommentModifyInput};
    use crate::services::test_support::{insert_normal_note, make_backend};

    use super::CommentService;

    #[tokio::test]
    async fn create_list_reply_and_modify_use_one_generic_contract() {
        let backend = make_backend().await;
        let note_id = insert_normal_note(&backend, "body", "ready").await;
        let service = CommentService::new(&*backend);
        let root = service.create(CommentCreateInput {
            note_id: note_id.clone(),
            block_text: "capture".to_string(),
            content: serde_json::json!({"kind":"project_route","destination":null,"probability":null}),
            author: "flick_jev".to_string(),
            parent_id: None,
            is_read: true,
        }).await.unwrap();
        let reply = service
            .create(CommentCreateInput {
                note_id: note_id.clone(),
                block_text: "capture".to_string(),
                content: serde_json::json!({"text":"reply"}),
                author: "Neil".to_string(),
                parent_id: Some(root.id.clone()),
                is_read: false,
            })
            .await
            .unwrap();
        assert_eq!(reply.parent_id.as_deref(), Some(root.id.as_str()));
        assert_eq!(service.list(&note_id).await.unwrap().len(), 2);
        let modified = service.modify(CommentModifyInput {
            id: root.id,
            content: Some(serde_json::json!({"kind":"project_route","destination":"none","probability":0.9})),
            is_read: Some(false),
        }).await.unwrap();
        assert_eq!(modified.content["destination"], "none");
        assert!(!modified.is_read);
    }

    #[tokio::test]
    async fn batch_modify_updates_all_comments() {
        let backend = make_backend().await;
        let note_id = insert_normal_note(&backend, "body", "ready").await;
        let service = CommentService::new(&*backend);
        let first = service
            .create(CommentCreateInput {
                note_id: note_id.clone(),
                block_text: "one".to_string(),
                content: serde_json::json!({"state":"old"}),
                author: "author".to_string(),
                parent_id: None,
                is_read: false,
            })
            .await
            .unwrap();
        let second = service
            .create(CommentCreateInput {
                note_id,
                block_text: "two".to_string(),
                content: serde_json::json!({"state":"old"}),
                author: "author".to_string(),
                parent_id: None,
                is_read: false,
            })
            .await
            .unwrap();

        let updated = service
            .modify_batch(CommentBatchModifyInput {
                comments: vec![
                    CommentModifyInput {
                        id: first.id,
                        content: Some(serde_json::json!({"state":"new"})),
                        is_read: None,
                    },
                    CommentModifyInput {
                        id: second.id,
                        content: None,
                        is_read: Some(true),
                    },
                ],
            })
            .await
            .unwrap();

        assert_eq!(updated[0].content["state"], "new");
        assert!(updated[1].is_read);
    }
}
