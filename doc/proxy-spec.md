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

## Proxy owner controls

The proxy owner configures their room through linggen settings or CLI:

- **Models exposed**: which of their configured models are available to consumers.
- **Max consumers**: 1–5 concurrent users.
- **Token budget**: daily or monthly cap.
- **Room type**: open or closed.
- **Allowed users** (closed rooms): invite list.
- **Auto-pause**: pause room when proxy owner is actively using linggen.
- **Session persistence**: whether consumer chat history is saved on the proxy.

## Infrastructure

### What's new on linggen.dev (CF Worker + D1)

- **Rooms table**: room_id, proxy_user_id, instance_id, model, region, max_consumers, current_consumers, token_budget, tokens_used, open, online.
- **Room browser page**: list open rooms, filter by model/region.
- **Dispatcher endpoint**: auto-match consumer to best available room.
- **Credit ledger**: per-user credit balance, transaction log (earn/spend).
- **Signaling**: same relay as remote access, but routes to room's instance instead of user's own instance.

### What's new on linggen server (Rust)

- **Room mode**: linggen server can advertise itself as a room proxy.
- **Consumer session isolation**: consumer sessions are sandboxed — no access to proxy owner's files, projects, or sessions.
- **Token counting**: track tokens per consumer session, report to linggen.dev for credit accounting.
- **Concurrent session management**: handle multiple consumer WebRTC connections.

### What's reused

- WebRTC transport (data channels, signaling relay).
- Session system (multi-session support).
- Control channel RPC (chat, API proxy).
- Instance registration and heartbeat.

## Implementation phases

### Phase 6a: Closed rooms (family sharing)

Proxy owner creates a closed room, invites specific users. Invited users connect and chat. No credits, no auto-dispatch — just direct sharing with trusted people.

This validates the core proxy flow: consumer connects to someone else's linggen and chats.

### Phase 6b: Open rooms + room browser

Open rooms visible on linggen.dev. Room browser page. Any user can join open rooms. Clear privacy warnings.

### Phase 6c: Credits + auto-dispatch

Credit system: earn by proxying, spend by consuming. Starter credits for new users. Auto-dispatch matches consumers to best available proxy.

This is the "zero-setup onboarding" milestone — new users chat immediately.

### Phase 6d: Polish

Usage dashboard for proxy owners. Abuse reporting. Proxy owner analytics (tokens shared, credits earned). Priority dispatch for high-credit users.
