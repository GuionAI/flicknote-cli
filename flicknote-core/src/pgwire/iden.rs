use sea_query::IdenStatic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, IdenStatic)]
#[iden(rename = "notes")]
pub(super) enum Notes {
    Table,
    Id,
    UserId,
    Type,
    Status,
    Title,
    Content,
    Summary,
    IsFlagged,
    ProjectId,
    Metadata,
    Source,
    ExternalId,
    CreatedAt,
    UpdatedAt,
    DeletedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IdenStatic)]
#[iden(rename = "projects")]
#[allow(dead_code)]
pub(super) enum Projects {
    Table,
    Id,
    UserId,
    Name,
    Color,
    PromptId,
    KeytermId,
    IsArchived,
    CreatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IdenStatic)]
#[iden(rename = "note_extractions")]
pub(super) enum NoteExtractions {
    Table,
    NoteId,
    Type,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IdenStatic)]
#[iden(rename = "prompts")]
pub(super) enum Prompts {
    Table,
    Id,
    UserId,
    Title,
    Description,
    Prompt,
    CreatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IdenStatic)]
#[iden(rename = "keyterms")]
pub(super) enum Keyterms {
    Table,
    Id,
    UserId,
    Name,
    Description,
    Content,
    CreatedAt,
    UpdatedAt,
}
