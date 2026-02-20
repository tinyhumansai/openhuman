# AlphaHuman Teams API Reference

Complete reference for all teams-related API endpoints in the AlphaHuman platform.

**Base URL**: `https://api.alphahuman.xyz`
**Authentication**: Bearer JWT token required for all endpoints
**Content-Type**: `application/json`

---

## Core Team Management

### 1. Create Team

**POST** `/teams`

Creates a new team with optional encryption.

**Request Body:**

```json
{ "name": "string (required)", "magicWord": "string (optional)" }
```

**Response Schema:**

```json
{
  "id": "string",
  "name": "string",
  "slug": "string",
  "magicWord": "string | null",
  "createdBy": "string",
  "isPersonal": "boolean",
  "subscription": {
    "hasActiveSubscription": "boolean",
    "plan": "FREE|BASIC|PRO",
    "planExpiry": "date-time | null",
    "stripeCustomerId": "string | null"
  },
  "usage": {
    "weeklyBudgetUsd": "number",
    "spentThisWeekUsd": "number",
    "weekStartDate": "date-time"
  },
  "inviteCode": "string | null",
  "maxMembers": "number",
  "createdAt": "date-time",
  "updatedAt": "date-time"
}
```

**Status Codes:**

- `200` - Team created successfully
- `401` - Unauthorized

---

### 2. List Teams

**GET** `/teams`

Retrieves all teams the authenticated user belongs to.

**Response Schema:**

```json
{
  "success": true,
  "data": [
    {
      "team": {
        "id": "string",
        "name": "string",
        "slug": "string",
        "isPersonal": "boolean",
        "subscription": { "hasActiveSubscription": "boolean", "plan": "FREE|BASIC|PRO" }
      },
      "role": "admin|billing_manager|member"
    }
  ]
}
```

**Status Codes:**

- `200` - Success
- `401` - Unauthorized

---

### 3. Get Team Details

**GET** `/teams/{teamId}`

Retrieves detailed information for a specific team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Response Schema:**

```json
{
  "id": "string",
  "name": "string",
  "slug": "string",
  "magicWord": "string | null",
  "createdBy": "string",
  "isPersonal": "boolean",
  "subscription": {
    "hasActiveSubscription": "boolean",
    "plan": "FREE|BASIC|PRO",
    "planExpiry": "date-time | null",
    "stripeCustomerId": "string | null"
  },
  "usage": {
    "weeklyBudgetUsd": "number",
    "spentThisWeekUsd": "number",
    "weekStartDate": "date-time"
  },
  "inviteCode": "string | null",
  "maxMembers": "number",
  "createdAt": "date-time",
  "updatedAt": "date-time"
}
```

**Status Codes:**

- `200` - Success
- `403` - Not a member of the team
- `404` - Team not found

---

### 4. Update Team Settings

**PUT** `/teams/{teamId}`

Updates team settings. **Admin only**.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Request Body:**

```json
{ "name": "string (optional)", "maxMembers": "number (optional)" }
```

**Status Codes:**

- `200` - Team updated successfully
- `403` - Only admins can update team settings

---

### 5. Delete Team

**DELETE** `/teams/{teamId}`

Deletes a team. **Admin only, non-personal teams only**.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Status Codes:**

- `200` - Team deleted successfully
- `400` - Cannot delete a personal team
- `403` - Only admins can delete a team

---

### 6. Switch Active Team

**POST** `/teams/{teamId}/switch`

Changes the user's active team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Status Codes:**

- `200` - Successfully switched active team
- `403` - Not a member of the specified team

---

## Team Member Management

### 7. List Team Members

**GET** `/teams/{teamId}/members`

Lists all members of a team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Response Schema:**

```json
{
  "success": true,
  "data": [{ "user": "string", "role": "admin|billing_manager|member", "joinedAt": "date-time" }]
}
```

**Status Codes:**

- `200` - Success
- `403` - Not a member of this team

---

### 8. Remove Team Member

**DELETE** `/teams/{teamId}/members/{userId}`

Removes a member from the team. **Admin only**.

**Parameters:**

- `teamId` (path, required) - Team identifier
- `userId` (path, required) - User identifier to remove

**Status Codes:**

- `200` - Member removed
- `403` - Only admins can remove members

---

### 9. Change Member Role

**PUT** `/teams/{teamId}/members/{userId}/role`

Changes a member's role within the team. **Admin only**.

**Parameters:**

- `teamId` (path, required) - Team identifier
- `userId` (path, required) - User identifier

**Request Body:**

```json
{ "role": "admin|billing_manager|member" }
```

**Status Codes:**

- `200` - Role updated
- `403` - Only admins can change member roles

---

### 10. Leave Team

**POST** `/teams/{teamId}/leave`

Allows a user to leave a team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Status Codes:**

- `200` - Successfully left the team
- `400` - Cannot leave as the only admin

---

## Team Invite Management

### 11. Create Team Invite

**POST** `/teams/{teamId}/invites`

Creates a new invite code for the team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Request Body (Optional):**

```json
{ "maxUses": "number (default: 1)", "expiresInDays": "number (default: 7)" }
```

**Response Schema:**

```json
{
  "success": true,
  "data": { "code": "string (e.g., T-1A2B3C4D5E6F)", "expiresAt": "date-time", "maxUses": "number" }
}
```

**Status Codes:**

- `200` - Invite created successfully
- `403` - Not a team member

---

### 12. List Team Invites

**GET** `/teams/{teamId}/invites`

Lists all invites for a team.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Status Codes:**

- `200` - Success
- `403` - User is not a member of the team

---

### 13. Revoke Team Invite

**DELETE** `/teams/{teamId}/invites/{inviteId}`

Revokes a team invite. **Admin or invite creator only**.

**Parameters:**

- `teamId` (path, required) - Team identifier
- `inviteId` (path, required) - Invite identifier

**Status Codes:**

- `200` - Invite successfully revoked
- `403` - Only admins or invite creator can revoke
- `404` - Invite does not exist

---

### 14. Join Team

**POST** `/teams/join`

Joins a team using an invite code.

**Request Body:**

```json
{ "code": "string (required, e.g., T-1A2B3C4D5E6F)" }
```

**Response Schema:**

```json
{ "success": true, "data": { "team": "string", "membership": "string" } }
```

**Status Codes:**

- `200` - Joined the team successfully
- `400` - Invite expired, max uses reached, or already a team member
- `404` - Invite code not found

---

## Team Billing Management

### 15. Purchase Team Subscription

**POST** `/teams/{teamId}/billing/purchase`

Purchases a subscription plan for the team. **Admin or billing manager only**.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Request Body:**

```json
{
  "plan": "BASIC_MONTHLY|BASIC_YEARLY|PRO_MONTHLY|PRO_YEARLY",
  "successUrl": "string (optional)",
  "cancelUrl": "string (optional)"
}
```

**Status Codes:**

- `200` - Checkout session created successfully
- `403` - Only admins or billing managers can purchase plans

---

### 16. Get Team Subscription Plan

**GET** `/teams/{teamId}/billing/plan`

Retrieves the team's current subscription information.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Response Schema:**

```json
{
  "success": true,
  "data": {
    "plan": "FREE|BASIC|PRO",
    "hasActiveSubscription": "boolean",
    "planExpiry": "date-time | null"
  }
}
```

**Status Codes:**

- `200` - Success
- `401` - Unauthorized

---

### 17. Create Billing Portal Session

**POST** `/teams/{teamId}/billing/portal`

Creates a Stripe billing portal session for subscription management. **Admin or billing manager only**.

**Parameters:**

- `teamId` (path, required) - Team identifier

**Request Body (Optional):**

```json
{ "returnUrl": "string" }
```

**Response Schema:**

```json
{ "success": true, "data": { "url": "string" } }
```

**Status Codes:**

- `200` - Portal session created successfully
- `403` - Only admins or billing managers can create portal session

---

## Team Roles

### Role Hierarchy

1. **admin** - Full team management permissions
2. **billing_manager** - Billing and subscription management
3. **member** - Basic team member access

### Permission Matrix

| Action               | Admin | Billing Manager | Member |
| -------------------- | ----- | --------------- | ------ |
| View team details    | ✅    | ✅              | ✅     |
| Update team settings | ✅    | ❌              | ❌     |
| Delete team          | ✅    | ❌              | ❌     |
| Add/remove members   | ✅    | ❌              | ❌     |
| Change member roles  | ✅    | ❌              | ❌     |
| Create invites       | ✅    | ✅              | ✅     |
| Manage billing       | ✅    | ✅              | ❌     |
| Leave team           | ✅\*  | ✅              | ✅     |

\*Admin cannot leave if they are the only admin

---

## Team Plans & Limits

### Plan Types

- **FREE** - Basic team functionality
- **BASIC** - Enhanced features and limits
- **PRO** - Full feature set and highest limits

### Usage Tracking

Teams have usage limits tracked through:

- `weeklyBudgetUsd` - Weekly spending budget
- `spentThisWeekUsd` - Current week spending
- `weekStartDate` - When the current week started
- `maxMembers` - Maximum team size

---

## Error Handling

### Common Error Responses

```json
{ "success": false, "error": "Error message description" }
```

### Authentication

All endpoints require a Bearer JWT token:

```
Authorization: Bearer <your-jwt-token>
```

### Rate Limiting

Standard API rate limits apply to all endpoints. Refer to main API documentation for specific limits.
