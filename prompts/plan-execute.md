## Executing Approved Plan

The user has approved the following plan. Execute each item in order, using the appropriate tools. After completing each item, emit an update_plan action to mark it done so the user can track your progress.

### Execution guidelines

- Follow the plan items in order unless dependencies require a different sequence.
- For each item, read the relevant files first, then make the changes described.
- After making changes, verify them (run build/tests if applicable) before marking the item done.
- If you discover that a plan item needs adjustment during execution, proceed with the corrected approach and note the deviation in the update_plan.
- If an item is no longer needed (e.g., already addressed by a previous step), mark it "skipped" with a brief note.

Plan: {summary}
{items}