---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Proxy Rooms

Community model sharing — users with API keys (proxies) share spare capacity with other users (consumers) through rooms. Consumers get free AI access; proxies earn credits. Built on linggen's existing WebRTC infrastructure.

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

### Non-goals

- Payment processing or real money exchange.
- End-to-end encryption (proxy must see messages to call the AI provider).
- Mesh topology between consumers — consumers only talk to the proxy.
- Guaranteed uptime or SLA for proxy rooms.

## Architecture

### Topology

Each room has exactly one proxy (the linggen server with API keys) and up to 5 consumers. Star topology — all consumers connect to the proxy via WebRTC data channels. Consumers do not connect to each other.

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

## Consumer modes

Two ways consumers can use a proxy room:

### Browser mode

Consumer opens linggen.dev, connects via WebRTC, and the owner's linggen UI is tunneled through the data channel. The consumer gets the full chat experience (markdown rendering, tool activity, skill results) running in their browser — but restricted by the owner's permission settings.

No separate consumer chat page needed. The owner's UI detects consumer mode (via `consumer_mode` message on the control channel) and hides irrelevant panels (settings, file browser, project selector).

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

- **rooms table**: id, owner_id, instance_id, name, room_type (private/public), invite_token, max_consumers, token_budget_daily, tokens_used_today.
- **room_members table**: room_id, user_id, consumer_type (browser/linggen).
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

### Phase 6b: Linggen server proxy mode — DONE

- Outbound WebRTC client (proxy_client.rs): consumer's linggen initiates connection to owner via relay signaling.
- Inference endpoint on owner (peer.rs): list_models + inference data channel handlers, filters by shared_models.
- Proxy model provider (proxy_provider.rs): ProviderClient::Proxy variant, streams StreamChunk over WebRTC, request/response demuxing.
- Auto-configure: connect_proxy_room discovers models, registers as proxy:id with owner name.
- Model selector: proxy models show "gpt-5.4 (By Tom)" in the UI.

### Phase 6c: UI polish + integration testing

- Consumer UI mode: detect consumer_mode on control channel, hide settings/tools/project panels, show privacy banner.
- Sharing tab: tool permission presets (chat/read/edit) with individual checkboxes.
- End-to-end testing: create room → join → connect → chat with tools → token budget → permission enforcement.
- Deploy DB migration + CF Worker + linggensite.

### Phase 6d: Credits + auto-dispatch

Credit system: earn by proxying, spend by consuming. Starter credits for new users. Auto-dispatch matches consumers to best available proxy.

### Phase 6e: Polish

Usage dashboard for proxy owners. Abuse reporting. Proxy owner analytics (tokens shared, credits earned). Priority dispatch for high-credit users. Cancellation token for clean proxy disconnect.
