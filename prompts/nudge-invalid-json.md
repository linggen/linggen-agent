Your previous response could not be parsed as valid JSON actions. This happens when the response includes plain text outside of JSON objects, malformed JSON, or uses a format the parser doesn't recognize.

Please respond with ONLY valid JSON objects matching the Response Format in the system prompt. Do not include any text, explanation, or markdown outside of the JSON.

Correct format examples:

Tool call:
{"name": "Read", "args": {"path": "src/main.rs"}}

Done:
{"type": "done", "message": "Task completed."}

Multiple actions in one response (each on its own line):
{"name": "Glob", "args": {"pattern": "src/**/*.rs"}}
{"name": "Read", "args": {"path": "Cargo.toml"}}

Your raw response was:
{raw}