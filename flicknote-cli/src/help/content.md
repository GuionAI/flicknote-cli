Print actual stored note content. This machine read does not synthesize title,
topics, project, or other managed editable-document frontmatter.

Examples:
  flicknote content 123
  flicknote detail 123 --tree
  flicknote content 123 --section 3K

Section IDs come from `flicknote detail <id> --tree`.
Output is markdown content only, suitable for piping to other tools.
