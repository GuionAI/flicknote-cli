Examples:
  flicknote find "API"
  flicknote find "API" "REST"
  flicknote find "::topic::AI::person::瓜子"
  flicknote find "whisper" "::topic::ASR::company::OpenAI"
  flicknote find "keyword" --project work
  flicknote find "keyword" --archived
  flicknote find "keyword" --limit 50
  flicknote find "keyword" --json

Multiple keywords use OR matching across title, content, and summary.
Structured filters use ::type::value pairs and are matched with AND logic.
