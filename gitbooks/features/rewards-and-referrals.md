---
description: >-
  Invite friends, earn referral credit, and unlock Discord community roles in
  OpenHuman. Promo-code redemption lives in the hosted billing dashboard.
icon: gift
---

# Rewards & Referrals

OpenHuman's Rewards surface bundles a **referral program** (share a code, earn credit when friends convert) and a **community rewards** track (link Discord, unlock roles as you hit usage milestones). Promo coupons are redeemed in the hosted TinyHumans billing dashboard, and invite-code management lives on its own screen.

All of this requires a signed-in backend session. On a local-only session the Rewards page shows an empty state prompting you to sign in - none of these features work offline.

---

## The Rewards screen

Lives at `/rewards` with two sidebar destinations. **Rewards** (community) is selected by default.

| Tab           | What it does                                                      |
| ------------- | ----------------------------------------------------------------- |
| **Referrals** | Your referral code, earnings, and referred-user activity          |
| **Rewards**   | Discord connection, progress ring, and unlockable community roles |

---

## Referrals

Each account has a single **referral code**. Copy it, or use **Share** (native share sheet, falling back to clipboard) to send a prefilled message with your code and the app download link.

The tab shows four tiles: your code, **total earned** (USD), **pending referrals**, and **completed** referrals. Below them, an **activity table** lists each referred user (masked identity, e.g. `j***@gmail.com`), a status badge, the reward amount, and a timestamp.

| Referral status | Meaning                                            |
| --------------- | -------------------------------------------------- |
| Joined          | Referred user signed up but hasn't converted yet   |
| Completed       | Referred user converted - referral reward credited |
| Expired         | Relationship lapsed (reserved; backend-driven)     |

If you were referred by someone else and are still eligible, an **apply** form lets you enter their code. Eligibility (`canApplyReferral`) is decided by the backend - typically only users who haven't already subscribed or already applied a code can claim one. Once applied, the form is replaced by a confirmation of the linked code.

Reward amounts, conversion rules, and eligibility are all enforced **server-side**. The desktop core is a thin adapter here.

### Under the hood

The referral domain (`src/openhuman/hosted/referral/`) is a stateless RPC adapter, not business logic. It exists because the desktop WebView `fetch` can fail with a generic "Load failed" (CORS/TLS/WebKit), so these calls reuse the same server-side `reqwest` path as billing.

| RPC                  | Backend call           | Purpose                                                               |
| -------------------- | ---------------------- | --------------------------------------------------------------------- |
| `referral.get_stats` | `GET /referral/stats`  | Code, totals, and referred-user rows                                  |
| `referral.claim`     | `POST /referral/claim` | Apply a referral code (optional device fingerprint for abuse signals) |

Both fail closed with `no backend session token` when no session is stored.

---

## Coupons

Coupon redemption and redemption history now live at **TinyHumans Dashboard → Pay as you go**, alongside promotional-credit and top-up balances. OpenHuman's Settings → Billing screen shows the read-only account summary and opens that hosted dashboard for redemption and other billing changes.

The hosted dashboard lists each code, its reward amount, status, and redemption time. Redemption is either applied immediately or accepted as **pending** when it is conditional on a later action.

| Coupon status  | Meaning                                         |
| -------------- | ----------------------------------------------- |
| Applied        | Fulfilled - credit is on your account           |
| Pending action | Conditional coupon awaiting a triggering action |
| Redeemed       | Accepted, not yet fulfilled                     |

---

## Community rewards & Discord

The Rewards tab gamifies usage. A **progress ring** shows how many achievements you've unlocked out of the total, and a list of **roles & rewards** describes each milestone (some carry an optional USD credit). Status badges (current streak, cumulative tokens) summarize your activity at the bottom.

Rewards are delivered as **Discord roles**, so the tab is built around linking your Discord account:

1. **Connect Discord** runs an OAuth consent flow (`openhuman.auth.oauth_connect` with provider `discord`); on success the snapshot refreshes and shows your Discord username.
2. **Join Discord** opens the community server invite.
3. **Disconnect** unlinks the account (clears the stored Discord ID, idempotent).

Once linked, each unlocked achievement shows its Discord role-assignment state:

| Role status   | Meaning                                                 |
| ------------- | ------------------------------------------------------- |
| Assigned      | Role granted on the server                              |
| Pending       | Unlocked but the role hasn't been assigned yet          |
| Join to claim | Linked but not in the server - join to receive the role |

If you've unlocked a role-bearing achievement but haven't joined the server, a **claim banner** prompts you to join. Membership status is one of `member`, `not_in_guild`, `not_linked`, or `unavailable`.

> GitHub-based contributor rewards are a **separate** mechanism: a GitHub Actions workflow (`.github/workflows/contributor-rewards.yml`) that posts a Discord/merch invite comment when a contributor's first PR merges. It is not part of the in-app Rewards screen and uses no in-app GitHub OAuth.

---

## Invite codes

The **Invites** screen (`/invites`) is distinct from referral codes. It manages personal **invite codes** that gate new-user signup:

- **Redeem** - if you haven't been invited yet, enter an invite code to claim your spot.
- **Your invite codes** - a list of the codes issued to you. Each row shows the code (monospace), a copy button, and an **enabled/disabled** state. A code flips to disabled once its uses are exhausted (`currentUses >= maxUses`), and shows who claimed it.

Invite codes carry a `type` (`USER` or `CAMPAIGN`), `maxUses`/`currentUses` counters, and a `usageHistory` of who redeemed them and when.

---

## See also

- [Billing & usage](billing-and-usage.md) - where referral, coupon, and achievement credit gets spent.
- [Welcome](../README.md) - the documentation home.
