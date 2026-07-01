# Lumen — business plan & roadmap

> **Purpose of this document:** personal working strategy, not an investor deck.
> It exists to force a decision about what to build next and why, and to
> separate "technically MVP-complete" (§14 of `docs/DESIGN.md`, already
> exceeded) from "ready to hand to a stranger and ask them to embed it in their
> product" (not yet true). Revisit and edit this as reality corrects it —
> especially the pricing numbers and the ICP wording, which are hypotheses,
> not facts, until a real customer conversation confirms or breaks them.

---

## 1. Where this actually stands today

The existing docs undersell the current state. `docs/DESIGN.md` calls this an
"MVP scaffold"; in reality, by the time of writing, the following are built
and working, not just designed:

- Compiler → DEP → runtime → iframe embedding (the original MVP, as designed)
- **Web Component embedding** (Shadow DOM, no iframe) — was explicitly
  "out of scope for MVP" in `DESIGN.md` §14.2, now shipped
- **A control-plane web app** (TanStack Start + Better Auth) with a real
  **drag-and-drop dashboard builder** — was explicitly "out of scope for MVP,"
  now shipped
- **Multi-warehouse support** (Postgres + ClickHouse, both live locally) and a
  **pluggable semantic-layer seam** (Cube.dev working end-to-end; a dbt
  Semantic Layer adapter scaffolded) with a UI to switch the active connection
  live, no restart
- Usage metering with the full credit formula, implemented and adversarially
  reviewed
- An `edge/` crate scaffolded for Cloudflare Workers deployment (the "compile
  once, run at every POP" vision from `DESIGN.md` §15 is now partially real,
  not just aspirational)

**The honest read: engineering has run well ahead of customer validation.**
That's the single biggest thing this document needs to correct for. Solo,
nights-and-weekends, pre-launch is exactly the stage where it's easiest to
keep adding real, impressive capability (another warehouse, another semantic
layer, a nicer builder) instead of doing the uncomfortable thing — showing it
to five strangers and finding out if any of them would actually use it. §6
below is deliberately about *cutting*, not adding.

---

## 2. Target customer (ICP)

**"B2B SaaS companies embedding analytics for their own customers"** is the
right category but too wide to act on. Sharpen it:

> **Primary ICP: a B2B SaaS company that already runs Cube.dev or dbt Semantic
> Layer for its own internal BI, and is now being asked by its customers (or
> its own product team) for a "your data" view inside the product itself.**

Why this is the wedge, not just "any SaaS company":

- They've already paid the cost of modeling their data in a semantic layer —
  the single biggest reason embedded-analytics projects stall. Lumen only has
  to solve the *last mile* (compile, embed, secure, meter), not the modeling
  problem.
- The trigger moment is concrete and common: an internal Cube/dbt setup used
  by the company's own analysts, and a customer-facing product manager asking
  "can our customers see this in-app." That request currently gets answered
  three ways, all bad: (a) a bespoke internal build against the Cube REST API
  — real engineering cost and an ongoing RLS-correctness liability; (b) adopt
  Cube Cloud's own embedding SDK — viable, and the most direct competitor
  (§3); (c) buy a general embedded-BI platform (Luzmo, Explo, ToucanToco) that
  wants to *own* the semantic layer and warehouse connection, which is
  friction/migration cost for a team that already has one they like.
- It's a *small*, findable population — Cube.dev and dbt both have public
  Slack communities and GitHub visibility, so "who already uses this" is
  answerable without an ad budget, which matters enormously at solo/
  nights-and-weekends bandwidth.

Secondary ICP (don't chase yet, but the architecture already supports it):
teams with a dbt Semantic Layer but no embedding story at all. Same wedge,
one adapter behind, less proven (the dbt adapter is scaffolded, not
production-verified against a live dbt Cloud instance per its own code
comments).

---

## 3. Competitive landscape & differentiation

| Competitor | What they are | Why a Cube/dbt-SL shop might still pick Lumen |
|---|---|---|
| **Cube Cloud's own embedding** | The most direct threat — "why not just use what we already pay for" | Cube's embedding is a thin client over live queries; Lumen's compile step means a warm dashboard costs *only its layout floor* (§11 of DESIGN.md — 13→5→1 credits cold→warm→304), which is a real, demonstrable cost argument for a customer with heavy embed traffic. Lumen is also warehouse/semantic-layer-portable (Cube *or* dbt SL) where Cube Cloud is obviously Cube-only. |
| **Luzmo, Explo, ToucanToco, GoodData** | General embedded-BI platforms | They typically want to own the semantic model or connect directly to the warehouse. A team with an existing, working Cube/dbt setup faces real migration cost adopting any of these; Lumen adds a layer *on top of* what they already have. |
| **Metabase (embedded)** | Open-source BI with an embedding feature | Interprets the dashboard per request (their own admitted architecture); no compile step, no credit-based cost story, not RLS-forwarding-to-a-semantic-layer by design the way Lumen is. |
| **Build it in-house** | The real default competitor for most prospects | This is what Lumen has to beat most often, not another vendor. The pitch has to be "cheaper and safer than your own eng time," not "better than a competing SaaS." |

**The differentiated story to lead with:** *"You already trust Cube (or dbt)
to define your metrics. Lumen is the missing embedding layer — compiled,
so it's fast and cheap to run at scale; RLS-forwarding, so your existing
security model just works; and it doesn't lock you into one semantic layer."*

---

## 4. Business model

Usage-based on compute credits, not seats — this was already the intended
model (`DESIGN.md` §11, fully implemented: `widget_base + QUERY_BASE×N +
compute_ms/100 + egress/MiB`, adversarially reviewed and fixed for
double-counting). Keep it; it's a real, load-bearing product decision, not
just a pricing afterthought — the incentive it creates (cache-warm dashboards
are cheap, cold ones cost what they cost) is a genuine technical + business
alignment that's rare to have this early.

**What's missing is a customer-facing price**, since "credits" is an internal
unit right now. Starting hypothesis (validate with real conversations, do not
treat as fixed):

| Tier | Price | Included | Who |
|---|---|---|---|
| **Design partner (free)** | $0 | Unlimited credits, manual onboarding, direct Slack/email access to you | First 3-5 customers — the price of admission is their time and feedback, not their money, for the first 60-90 days |
| **Starter** | ~$99/mo | ~100k credits/mo, overage billed per 10k credits | Small teams past the design-partner phase |
| **Growth** | ~$399/mo | ~500k credits/mo, overage billed at a lower per-credit rate | Teams with real embed traffic |
| **Enterprise** | Custom | Committed volume, SSO, SLAs | Later — not a phase-1 concern |

The exact numbers matter far less right now than the *shape*: free for design
partners, metered after. Do not spend more cycles calibrating exact dollar
amounts until at least one real prospect has reacted to a price.

---

## 5. Must-fix before any real design partner (trust gate, not feature gate)

This is distinct from "features to build" — these are things that would
correctly scare off any technically-competent design partner doing five
minutes of due diligence, surfaced by the project's own adversarial review
(`docs/REVIEW-FINDINGS.md`) and confirmed still-open by re-checking the
runtime code:

1. **Dashboards are not tenant-scoped at the runtime.** `REVIEW-FINDINGS.md`
   flagged this explicitly ("any valid token can load any compiled
   dashboard's layout"), and it is *still true* even after this session's
   control-plane work — the new `dashboards` Postgres table is authoring-side
   only; `GET /embed/dashboard/{id}` never checks whether the calling
   tenant's token is actually authorized to view dashboard `{id}`. For a
   single-tenant demo this is invisible. For a design partner embedding this
   for *their own customers*, it's a cross-tenant data leak waiting to
   happen the moment two customers' dashboard ids are guessable or leaked.
   **This is the single highest-priority fix before showing this to anyone
   outside this room.**
2. **Dev-only secrets must not ship.** `LUMEN_JWT_SECRET`, `CUBEJS_API_SECRET`,
   Better Auth secrets — all currently default to insecure dev values.
   Trivial to fix, easy to forget under solo/nights-and-weekends pressure.
3. **Single-flight cache stampede** (deferred in the review, `moka::get_with`
   coalescing) — matters once a design partner has real concurrent traffic;
   not before.
4. **HS256 shared-secret JWTs → RS256/JWKS** — fine for a design partner you
   personally onboard by hand; becomes a real requirement the moment a
   partner wants to mint tokens from their own infrastructure without
   sharing a symmetric secret with you.

Items 1-2 are the actual gate. 3-4 can wait for the first partner's real
usage pattern to justify the work.

---

## 6. The real MVP — what to build vs. what to deliberately not build yet

Given the "engineering ahead of validation" problem in §1, the goal here is
mostly **subtraction**. What a first design partner conversation actually
needs:

**Keep / finish:**
- Cube.dev path only, end to end, rock solid (iframe *and* Web Component)
- The tenant-scoping fix (§5.1) — non-negotiable
- One clean onboarding path: partner gives you a Cube.dev URL + API secret →
  you (manually, by hand, for the first few) get their first dashboard
  compiled and embedded in under a day
- A single, honest pricing page reflecting §4's shape (even if the numbers
  are marked "early access")

**Deliberately do not invest further in right now:**
- **ClickHouse / multi-warehouse support.** Real, working, and a distraction —
  no design partner has asked for it yet. It cost real engineering time this
  session that could have gone toward the tenant-scoping fix instead. Keep it
  (it's already built, don't rip it out), but don't extend it further until a
  specific prospect asks.
- **dbt Semantic Layer adapter.** Scaffolded, unverified against a live dbt
  Cloud instance (flagged in its own code comments as untested end-to-end).
  Don't invest more here until a Cube-only launch has at least one real
  customer *and* a second prospect specifically says "we're on dbt, not
  Cube" — then it's a qualified reason to finish it, not speculative
  flexibility.
- **Drag-and-drop builder polish.** It works. Resist the pull to keep
  improving it (better resize handles, more widget types, live preview) until
  a real customer's dashboard-authoring workflow tells you what's actually
  missing. It's real product-differentiation work, but it's happening in a
  vacuum right now.
- **Cloudflare edge deployment.** The most impressive, most premature piece
  of what's built. "Runs at the edge" is a scaling/cost story that matters at
  meaningful traffic volume, which a pre-first-customer product does not
  have. Keep the scaffold, don't extend it.

The test for anything not in the "keep/finish" list: *would a specific,
identifiable prospect's "yes" or "no" hinge on this?* If not, it's deferred,
regardless of how technically satisfying it is to build.

---

## 7. Go-to-market (solo, nights-and-weekends)

No ad budget, no team, limited hours — the plan has to fit that.

1. **Find 15-20 named companies**, not a persona: search Cube.dev's public
   Slack, GitHub stars/forks of `cube-js/cube`, and dbt's public Slack for
   companies that are visibly *using* (not just evaluating) one of these
   tools and are themselves a B2B SaaS product (i.e., they have their own
   customers who'd want an embedded view).
2. **Direct outreach, not content marketing.** At this stage, five real
   conversations are worth more than a launch post. A specific, personalized
   message referencing their actual public Cube/dbt usage will out-convert
   generic outbound by a wide margin.
3. **Design partner offer:** free, white-glove, in exchange for a real
   embedded dashboard in their product and permission to use them as a
   reference once it's live. The ask is small (a Cube URL, one dashboard's
   worth of scope, direct access to you) and the value is immediate (working
   embedded analytics in days, not the weeks/months of an in-house build).
4. **Only after 2-3 design partners are live:** a public "Show HN" / Product
   Hunt style launch, anchored on the compiled-artifact/credit-cost story
   (§3's differentiation) — that story is genuinely novel enough in the
   embedded-BI space to earn attention on its own, but it's much stronger
   with "and here's a live customer" attached.

---

## 8. Risks, named honestly

- **"Build vs. buy vs. build-on-Cube's-own-embedding" is the real
  competitive fight**, not another embedded-BI vendor. Most prospects will
  seriously consider just using Cube Cloud's own embedding feature or having
  an engineer spend a sprint on it. The compiled/credit-cost story has to
  land in the first five minutes of any conversation, or the meeting is lost
  to "we'll just build it."
- **Usage-based pricing is a harder sell pre-trust than flat pricing.** A
  design partner with no track record of your reliability may prefer a flat
  number they can budget, not a variable one. Consider whether the first 1-2
  design partners should get a flat-fee pilot price *while* the credit meter
  runs in the background purely for your own data, converting to metered
  pricing once trust and predictability are established.
- **Solo bandwidth is the real constraint**, not technology. Every hour spent
  extending an already-sufficient technical surface (§6's "don't invest
  further" list) is an hour not spent finding out whether anyone will pay for
  what already exists.
- **The tenant-scoping gap (§5.1) is a real liability the day this touches a
  second customer**, not a theoretical one — it's the difference between "MVP
  cutting corners" and "a security incident in someone else's product."

---

## 9. Next 30/60/90 days

- **Days 1-14:** Fix the tenant-scoping gap (§5.1) and secrets hygiene
  (§5.2). Nothing else technical until this is done.
- **Days 1-14 (parallel):** Build the list of 15-20 named prospects (§7.1)
  and send the first 5 outreach messages. This can and should happen before
  the security fix is finished — conversations take days to get replies;
  don't gate outreach on code.
- **Days 15-45:** Land 1-3 design partners. Manually onboard each one
  (§6's "clean onboarding path") — don't build self-serve signup yet, that's
  premature for 1-3 customers.
- **Days 45-90:** With at least one dashboard live in a real partner's
  product, decide — with actual customer signal instead of guesswork —
  whether the next investment is the dbt adapter, self-serve onboarding, the
  builder's polish, or something not on this list at all that a real
  conversation surfaced. Re-read and correct this document at that point;
  most of §2's ICP wording and §4's price anchors should be sharper or wrong
  by then, and that's the plan working as intended.

---

## 10. Open questions this document can't answer alone

These need a real prospect conversation, not more solo thinking:

- Does "compiled = cheaper at scale" actually matter to a prospect who has no
  scale yet, or does it only land with someone who's already feeling Cube
  Cloud's embedding costs?
- Is the true buyer the same person who owns the Cube/dbt setup internally,
  or a separate product-team stakeholder who's never heard of Cube? (Changes
  the pitch entirely.)
- Would a design partner actually accept usage-based pricing, or does §8's
  "flat pilot price" hedge need to be the default, not a fallback?
