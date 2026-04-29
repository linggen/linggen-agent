---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Rooms

Community model sharing. An owner opens a **room** to share their models with other users (consumers); the owner's linggen acts as the proxy that calls the AI provider on the consumer's behalf. Consumers get AI access without setting up keys; owners earn credits. Built on linggen's existing WebRTC infrastructure.

## Related docs

- `webrtc-spec.md`: transport, signaling, data channels.
- `session-spec.md`: session lifecycle.
- `chat-spec.md`: event model, message types.

## Motivation

New linggen users face a cold-start problem: they need API keys before they can do anything. Keys cost money and require signup with providers. This friction kills adoption.

Meanwhile, existing users often have unused API capacity — paid subscriptions with idle tokens, local Ollama models sitting quiet. There's no way to share this surplus.

Proxy rooms solve both problems. Contributors share their linggen server as a proxy, earning credits. New users spend credits to chat — zero setup, no API keys needed. The experience feels like a cloud service, but it's powered by the community.

### Goals

- Zero-setup onboarding: new users chat immediately via community proxies.
- Reward contributors with credits (not money) — grows the ecosystem.
- Two trust levels: open rooms (anyone) and closed rooms (invited users).
- Auto-dispatch: users can be matched to available proxies automatically.
- Star topology: one proxy, multiple consumers — no mesh complexity.
- One room per user: a user is either an owner or a consumer at any time, never both simultaneously. Prevents proxy loops and simplifies the mental model.

### Non-goals

- Payment processing or real money exchange.
- End-to-end encryption (proxy must see messages to call the AI provider).
- Mesh topology between consumers — consumers only talk to the proxy.
- Guaranteed uptime or SLA for proxy rooms.
## Architecture

### Room ownership and membership

**One room per owner**: each user can own at most one room. To create a new room, delete the existing one first. `rooms.owner_id` is UNIQUE.

**Multiple rooms as consumer**: a user can join multiple rooms as a consumer (except their own room). This allows a user to be both an owner sharing their models AND a consumer using models from other rooms simultaneously.

- **Owner**: has a room created from their instance. Their linggen serves models to consumers.
- **Consumer**: joined one or more rooms. Uses owners' models (browser mode or linggen server mode).
- **A user cannot join their own room** — enforced at the API level.

**No re-sharing**: only local models (Ollama, OpenAI, etc.) can be shared. Proxy models received from other rooms are never eligible for sharing. Enforced in UI (Sharing tab hides proxy models) and server-side (list_models handler filters out proxy:* models). This prevents proxy loops (A shares to B, B re-shares to A).

**Leaving and removal**:
- Consumers can leave any room they've joined (Leave button).
- Room owners can remove any member from their room (Remove button).
- Deleting a room removes all members.

### Topology

Each room has exactly one proxy (the linggen server with API keys) and up to 4 consumers. Star topology — all consumers connect to the proxy via WebRTC data channels. Consumers do not connect to each other.

```
Consumer B ──┐
Consumer C ──┤── Proxy A (linggen server)
Consumer D ──┘
```

Each consumer gets an independent session on A's linggen. From A's perspective, it's the same as having multiple browser tabs open. The existing multi-peer support handles this.

### Connection flow

1. Proxy A creates a room on linggen.dev (or via `ling proxy create`).
2. A's linggen registers with the signaling relay as a room proxy.
3. Consumer B discovers the room (browse or auto-dispatch).
4. B connects to A via the signaling relay — same WHIP-style exchange as remote access.
5. B gets a session on A's linggen. Chat messages flow through A's server to the AI provider.

The proxy's linggen server handles all AI provider communication. Consumers never touch API keys.

## Room types

### Open rooms

Visible on linggen.dev's room browser. Anyone with a linggen account can join (costs credits). The proxy owner accepts that any community member may use their models.

Open rooms are the backbone of the "try linggen for free" experience. Auto-dispatch draws from the open room pool.

### Closed rooms

Invite-only. The proxy owner generates invite links or adds specific users. Only approved users can join. Not listed in the room browser.

Closed rooms are for family, teams, or friends. Higher trust — the proxy owner knows who's using their models.

## Discovery

### Manual browse

linggen.dev lists open rooms with:

- Available model(s)
- Open slots (e.g., "2/5 slots available")
- Region / estimated latency
- Remaining token budget
- Proxy uptime history

Users pick a room and click to connect.

### Auto-dispatch

User clicks "Start chatting" on linggen.dev — no room selection needed. The dispatcher picks the best available proxy:

1. Filter: open rooms, online, has available slots, has remaining budget.
2. Match: user's preferred model, closest region.
3. Sort: most budget remaining, lowest current load.
4. Connect: relay signaling to the chosen room's proxy.

From the user's perspective, it looks like a cloud service. They don't know or care which proxy serves them. If the proxy goes offline, the dispatcher reconnects them to another one.

## Credit system

Credits are the community currency — not real money, not transferable, not purchasable.

### Earning credits

Proxies earn credits based on tokens consumed through their rooms:

- 1 credit per 1K tokens proxied (input + output).
- Credits accumulate on the proxy owner's linggen.dev account.
- Only successful completions count (errors, timeouts don't earn).

### Spending credits

Consumers spend credits when using proxy rooms:

- Cost matches the earn rate: 1 credit per 1K tokens consumed.
- Balance shown before joining a room and during chat.
- When balance hits zero, session pauses with guidance to earn credits or use own keys.

### Starter credits

New linggen.dev accounts receive starter credits (e.g., 50K tokens worth) to try the service. Enough for a meaningful first experience.

### What credits unlock

Beyond proxy access, credits unlock perks:

- Higher instance limits (link more machines).
- Priority in auto-dispatch queue.
- Profile badges (contributor tiers).
- Future: Pro features without paid subscription.

### What credits are NOT

- Not real money — cannot be cashed out.
- Not transferable between users.
- Not purchasable — earned only by contributing.
- No exchange rate, no marketplace.

## Token budget

Proxy owners set a token budget per room to control costs:

- Daily or monthly budget (e.g., 500K tokens/month).
- When budget is exhausted, the room shows as "full" — no new sessions, existing sessions finish their current turn.
- Budget resets automatically on the configured cycle.
- Proxy owner can top up or adjust at any time.

Budget is enforced by A's linggen server (it controls the API calls). The CF dispatcher also tracks usage for credit accounting.

## Privacy and trust

### What the proxy sees

The proxy owner's linggen server processes all messages. This means:

- The proxy can see consumer messages and AI responses.
- This is inherent — the server must see the content to make API calls.
- This is clearly stated to consumers before joining any room.

### Trust model

| Room type | Trust level | Privacy expectation |
|:----------|:-----------|:-------------------|
| Closed | High (invited users) | Family/team — trusted context |
| Open | Low (strangers) | Use at own risk — don't share secrets |

### Mitigations

- Consumer sees a clear warning before joining open rooms: "The proxy owner can see your messages."
- Proxy owner agrees to a community conduct policy when creating a room.
- Abuse reporting: consumers can flag rooms, linggen.dev can delist.
- Session history is not persisted on the proxy by default for room consumers (configurable by proxy owner).

## Provider considerations

Proxy rooms use the proxy owner's API keys. This is technically sharing API access, which some providers restrict. Linggen's position:

- Credits are not money — this is community sharing, not reselling.
- Similar to a family sharing a Netflix account or a team sharing an API key.
- Closed rooms (family/team) are clearly within reasonable use.
- Open rooms are more grey — positioned as community generosity, not a service.
- Linggen does not encourage circumventing provider terms.
- If a provider objects, their models can be excluded from open rooms.
- Local models (Ollama) have no TOS concerns — ideal for open rooms.

## Room UI (linggen local Settings > Room page)

The Room page in linggen's local UI shows all room activity for the current user. Three sections: My Room (owner), Joined Rooms (consumer), Available Rooms (discovery).

### My Room (owner section)

If the user owns a room, shows full room management:
- Room info: name, type (private/public), online status, token usage bar.
- Editable fields: name, type, max consumers, daily budget.
- Invite link with copy + regenerate (private rooms).
- Shared models: checkboxes for local models only (proxy models hidden).
- Consumer permissions: tool presets (chat/read/edit) + individual tool checkboxes.
- Shared skills: checkboxes filtered by permission level.
- Members list: avatar, name, consumer type (browser/linggen), remove button.
- Delete Room button at the bottom.

If the user has no room, shows a "Create Room" button with a creation form.

Each user can own at most one room.

### Joined Rooms (consumer section)

Lists all rooms the user has joined as a linggen server consumer. Each entry shows:
- Room name, owner name, online status dot.
- Proxy model connection status: connected (with discovered model list) or disconnected.
- **Leave** button → leaves the room, removes proxy models from selector.

A user can join multiple rooms. Cannot join their own room. Browser mode consumers are managed on linggen.dev dashboard, not shown here.

### Available Rooms (discovery section)

**Public rooms**: listed for all visitors, but only logged-in users can join. Shows:
- Room name, owner, slots (e.g. "2/4"), budget, online status.
- "Join" button → joins as linggen server consumer type.
- Rooms the user already joined are shown with a "Joined" badge instead of "Join".

**Private rooms**: not listed. Users join via invite link.
- Invite link input field at the bottom: paste `https://linggen.dev/join/room_...` and click Join.
- Shows room preview (name, owner, slots) before confirming.

### Linggen server auto-connect

When a user joins a room with `consumer_type: "linggen"`:

1. Linggen server on startup calls `GET /api/rooms/joined` (proxied to linggen.dev).
2. For each joined room where `consumer_type: "linggen"` and the owner is online → auto-calls `connect_proxy_room(instance_id, owner_name)`.
3. Owner's shared models appear in the consumer's model selector as `gpt-4 (By Tom)`.
4. On disconnect (or owner goes offline), proxy models are removed from the selector.

Multiple proxy rooms can be connected simultaneously — models from all connected rooms appear in the selector.

No manual CLI command needed — joining the room on the dashboard is the only setup step.

## Consumer modes

Two ways consumers can use a proxy room:

### Browser mode

Consumer opens linggen.dev, connects via WebRTC, and the owner's linggen UI is tunneled through the data channel. The consumer gets the full chat experience (markdown rendering, tool activity, skill results) running in their browser — but restricted by the owner's room settings.

A dedicated consumer chat page shows the consumer-relevant UI: chat widget, session list, allowed skills, room name, privacy warning ("Owner can see your messages"), token budget, and leave button. Owner-only features (settings, file browser, project selector, mission editor) are not shown.

#### Permission enforcement

Consumer sessions cannot ask the consumer for additional permissions. All enforcement is server-side:

1. **Room config is the ceiling**: `room_config.toml` defines `allowed_tools` and `allowed_skills`. These are the hard limits regardless of the consumer's permission level.
2. **Permission level is a path-mode grant**: the consumer's permission (chat/read/edit) determines what the agent can do in the consumer session scope, but only within what the room config allows. Even a consumer with edit permission cannot use tools the owner hasn't shared.
3. **No consumer prompts**: consumer sessions never show permission prompts for tool access, mode upgrades, or path grants. If an action needs more permission, it is blocked or returned as permission-needed to the owner-side runtime.
4. **System prompt filtering**: the agent's tool list and skill list only include what the room config allows. The agent never sees blocked tools, so it won't attempt to call them.
5. **Execution gate**: defense-in-depth — if the agent hallucinates a blocked tool or skill call, the engine blocks it at execution time.
6. **Skill trigger enforcement**: consumers cannot bypass skill restrictions by typing `/skill-name` directly. Both button-click and trigger-command paths check the allowlist.

### Linggen server mode

Consumer runs their own linggen instance and uses the owner's linggen as a remote model provider. The consumer's agent engine runs locally with tools, but inference is proxied through WebRTC to the owner. Consumer's files stay local — the owner only sees inference requests.

Proxy models appear in the consumer's model selector with the owner's name: `gpt-5.4 (By Tom)`.

## Consumer permissions

The owner controls what consumers can do via the Sharing tab in local settings. Permissions are stored in `~/.linggen/room_config.toml` and loaded server-side — consumers cannot override them.

### Tool allowlist

Owner selects which tools are available to consumers. Preset modes for convenience:

| Preset | Tools allowed |
|:-------|:-------------|
| Chat only | None — conversation only |
| Read (default) | WebSearch, WebFetch |
| Edit | WebSearch, WebFetch, Read, Write, Edit, Glob, Grep, Bash |

Owner can fine-tune individual tools beyond presets. For trusted consumers (family), the owner can enable all tools including Bash.

### How it's enforced

1. **System prompt**: only allowed tools appear in the agent's tool list. The agent never sees blocked tools, so it won't attempt to call them.
2. **Execution gate**: defense-in-depth — if the agent hallucates a blocked tool call, the engine blocks it at execution time.
3. **Server-side only**: permissions are loaded from `room_config.toml` by the server. The HTTP API and WebRTC layer cannot inject or override permissions.

### Skill allowlist

Owner selects which skills consumers can trigger. Default: none. Owner adds specific skills (e.g. weather, translator).

### Shared models

Owner selects which of their configured models consumers can use. Default: none. The `list_models` and `inference` data channel handlers only serve shared models.

## Proxy owner controls

The proxy owner configures their room through linggen Settings > Sharing tab:

- **Shared models**: which of their configured models are available to consumers.
- **Allowed tools**: which tools consumers can use (preset modes + individual checkboxes).
- **Allowed skills**: which skills consumers can trigger.
- **Max consumers**: 1–4 concurrent users.
- **Token budget**: daily cap.
- **Room type**: public (anyone) or private (invite only).
- **Instance**: which of their instances powers the room (one instance per room).
- **Allowed users** (private rooms): invite link, member list with remove.

Room metadata (name, type, members, budget) is stored on linggen.dev (D1). Local-only settings (shared models, allowed tools/skills) are stored in `~/.linggen/room_config.toml`.

## Auth and verification

### Three-layer verification

```
Layer 1: SIGNALING (linggen.dev CF Worker)
  ├─ Consumer authenticated by session cookie (browser) or API token (linggen server)
  ├─ Room membership verified in DB (room_members table)
  ├─ Consumer metadata (type, budget) attached to signaling payload from DB
  └─ Consumer cannot fake their type or budget — set by the relay from DB

Layer 2: OWNER'S LINGGEN (peer connection)
  ├─ Receives ConsumerContext from signaling envelope
  ├─ list_models → only returns owner's shared_models
  ├─ inference → rejects models not in shared_models
  ├─ Token budget enforcement (rejects if exhausted)
  └─ Sends consumer_mode message so UI adapts

Layer 3: ENGINE (per-request)
  ├─ mode="consumer" triggers server-side permission loading
  ├─ Allowed tools loaded from room_config.toml (not from request body)
  ├─ System prompt built with only allowed tools — agent never sees blocked tools
  └─ Execution gate blocks hallucinated tool calls as defense-in-depth
```

### Consumer identity

- Browser consumers: identified by linggen.dev session cookie (JWT)
- Linggen server consumers: identified by API token (from `ling login`)
- Both verified against `room_members` table at the signaling layer
- Room membership established at join time (invite link or public join)
- Owner can remove members at any time — next connection attempt fails

## Infrastructure

### What's on linggen.dev (CF Worker + D1)

- **rooms table**: id, owner_id (UNIQUE — one room per owner), instance_id (UNIQUE), name, room_type (private/public), invite_token, max_consumers, token_budget_daily, tokens_used_today.
- **room_members table**: room_id, user_id, consumer_type (browser/linggen). UNIQUE(room_id, user_id) — no duplicate membership per room, but a user can join multiple rooms.
- **Room API**: create, update, delete, join (invite token or public), leave, regenerate invite, list public rooms, preview room by invite token.
- **Room browser page** (`/rooms`): list public rooms with join button.
- **Join page** (`/join/{token}`): invite landing page with consumer type selection and privacy warning.
- **Dashboard**: owner room management (create/edit/members/invite) + joined rooms list for consumers.
- **Signaling**: same relay as remote access. Authenticates both cookie (browser) and bearer token (linggen server) consumers. Attaches consumer metadata from DB.

### What's on linggen server (Rust)

- **Room config** (`~/.linggen/room_config.toml`): shared_models, allowed_tools, allowed_skills.
- **Inference endpoint**: `list_models` and `inference` data channel handlers for linggen server proxy mode. Filters by shared_models. Streams StreamChunk tokens back.
- **Outbound WebRTC client** (`proxy_client.rs`): consumer's linggen initiates WebRTC to owner's linggen via relay signaling.
- **Proxy model provider**: new `ProviderClient::Proxy` variant. Sends inference over WebRTC data channel, yields StreamChunk stream. Registered with `proxy:` prefix + owner name.
- **Consumer permission enforcement**: mode="consumer" triggers server-side loading of allowed tools/skills from room_config. Filters system prompt and execution gate.
- **Room API proxy**: `/api/rooms/*` proxied to linggen.dev with API token. Auto-injects instance_id.
- **Settings > Sharing tab**: room management, shared models checkboxes, allowed tools presets.

### What's reused

- WebRTC transport (data channels, signaling relay, tunnel loading).
- Session system (multi-session support, each consumer gets independent session).
- Control channel RPC (chat, API proxy).
- Instance registration and heartbeat.
- Permission system (EngineConfig.effective_tool_restrictions for cascading intersections).

## Implementation phases

### Phase 6a: Private rooms + browser consumer — DONE

- DB: rooms + room_members tables, room_type (private/public), invite tokens.
- CF Worker API: room CRUD, join/leave, signaling with consumer auth (cookie + bearer token).
- Rust backend: ConsumerContext on peer connections, consumer permission enforcement, room_config.toml (shared models, allowed tools).
- linggen.dev frontend: dashboard room sections, join page, public rooms browser.
- linggen local UI: Settings > Sharing tab (room management, shared models, member list).
- Browser consumer mode: tunnel owner's UI via WebRTC, consumer_mode message on control channel, server-side permission loading.
- Consumer permission enforcement: SessionPolicy loads room_config for consumer sessions (allowed_tools, allowed_skills, consumer path-mode ceiling). System prompt filters skills. Execution gate blocks disallowed tool/skill calls. Consumer sessions cannot approve new path grants.

### Phase 6b: Linggen server proxy mode — DONE

- proxy_client.rs: outbound WebRTC connection, SDP exchange via relay, data channel lifecycle.
- proxy_provider.rs: ProviderClient::Proxy variant, streams inference over WebRTC, request demuxing.
- peer.rs: list_models and inference data channel handlers. Filters by shared_models, rejects proxy:* models, enforces token budget.
- One-room-per-owner: rooms.owner_id UNIQUE in DB.
- Multiple consumer rooms: room_members UNIQUE(room_id, user_id), user can join multiple rooms. Cannot join own room (UI shows "Yours" badge).
- No re-sharing: list_models handler filters out provider="proxy" models. Sharing tab hides proxy models. Prevents proxy loops.
- Auto-connect on startup: linggen server calls GET /api/rooms/joined, auto-connects to rooms where consumer_type=linggen and owner is online.
- Room page in local UI: My Room (owner CRUD + config), Joined Rooms (consumer list with connect/disconnect/leave), Available Rooms (public rooms + invite link input).
- Model selector: proxy models show "gpt-4 (By Tom)" via provided_by field.
- Note: server mode consumers run their own engine locally — permission enforcement is not needed. They only use the owner for inference (list_models + inference endpoints).

### Phase 6c: UI polish — DONE

- Consumer UI mode: consumer_mode on control channel, restricted UI for browser consumers.
- Sharing tab: tool permission presets (chat/read/edit) with individual tool checkboxes.
- Deployed: DB migration + CF Worker + linggensite.

### Phase 6d: Credits + auto-dispatch

Credit system: earn by proxying, spend by consuming. Starter credits for new users. Auto-dispatch matches consumers to best available proxy.

### Phase 6e: Polish

Usage dashboard for proxy owners. Abuse reporting. Proxy owner analytics (tokens shared, credits earned). Priority dispatch for high-credit users. Cancellation token for clean proxy disconnect.
