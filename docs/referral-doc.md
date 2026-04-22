# Referral System

## Overview

Link-based referral system with one-time flat rewards for both parties.

- Reward: **$5** credit to referrer and **$5** credit to referred user
- Reward is one-time: awarded when the referred user's first subscription payment is confirmed
- No recurring rewards — once a referral is marked `CONVERTED`, subsequent payments return `already_converted`

## Main Rules

- Each user has one unique referral code with a shareable link.
- A user can claim a referral link only before their first subscription.
- Self-referral is blocked (user id + identity fields).
- Rewarding is idempotent:
  - same payment cannot reward twice
  - only one reward per referral (always one-time)

## Data Model

### `ReferralCode` (`referralcodes`)

- `userId` (unique)
- `referralCode` (unique)

### `Referral` (`referrals`)

- `referrerId`
- `referredUserId` (unique)
- `referralCode`
- `status`: `pending | converted`
- `sourceIp`, `deviceFingerprint`, `convertedAt`

### `ReferralTransaction` (`referraltransactions`)

- `referralId`, `referrerId`, `referredUserId`
- `sourcePaymentId`, `sourcePaymentGateway`, `sourcePaymentObjectId`
- `paymentAmountUsd`, `rewardAmountUsd`, `rewardRate` (Decimal128)
- `creditTransactionId`
- `recipientType`: `REFERRER | REFERRED`
- `idempotencyKey` (unique)

## Migration

Migration file: `src/migrations/1744200000000-referral-system.ts`

What it does:

- creates indexes for referral collections
- backfills missing referral codes for existing users
- backfills `Referral` records from legacy `user.referral.invitedBy`
- supports `users` and `tgusers` collections

### Run migration (non-interactive)

```bash
npx ts-migrate-mongoose up -f src/migrate.ts -a true
```

### Check migration status

```bash
npm run migrate:list
```

### Roll back referral migration (if needed)

```bash
npm run migrate:down
```

## Core Services

- `src/services/referral/referralCodeService.ts`
  - ensures and fetches user referral codes
- `src/services/referral/referralService.ts`
  - claim referral link, enforce eligibility (subscription-based gating), return referral stats
- `src/services/referral/referralRewardService.ts`
  - award flat $5 credit to both referrer and referred user, upsert audit transactions, mark referral converted

## API

- `GET /referral/stats`
  - returns code, referral link, totals, and referral rows
- `POST /referral/claim`
  - request: `{ "code": "ABCD1234", "deviceFingerprint": "optional" }`
  - supports `x-device-fingerprint` header
  - only users who have never subscribed are eligible

## Payment Integration

Reward processing is triggered on successful payment flows in:

- `src/controllers/payment/coinbase/webhook.ts`
- `src/controllers/payment/stripe/handleWebhook.ts`

## Tests

Key test: `src/services/referral/__tests__/referralRewardService.test.ts`

- flat $5 reward to both parties
- conversion blocking (already_converted)
- payment idempotency
- partial-retry recovery
