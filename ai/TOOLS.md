# OpenHuman Tools

This document lists all available tools that OpenHuman can use to interact with external services and perform actions. Tools are organized by integration and automatically updated when the app loads.

> **Architecture note**: All read/query operations (get-page, list-\*, query-database, search, etc.) are handled by the memory layer — data is fetched from the TinyHumans Neocortex memory system and injected into context automatically. Only write, create, update, delete, and trigger operations are exposed as tools.

## Overview

OpenHuman has access to **12 tools** across **1 integrations**.

**Quick Statistics:**

- **Notion**: 12 tools

## Available Tools

### Notion Tools

This skill provides 12 tools for notion integration. Data retrieval is handled by the memory layer, not tools.

#### append-blocks

**Description**: Append child blocks to a page or block. Supports various block types.

**Parameters**:

- **block_id** (string) **(required)**: The parent page or block ID
- **blocks** (string) **(required)**: JSON string of blocks array. Example: [{"type":"paragraph","paragraph":{"rich_text":[{"text":{"content":"Hello"}}]}}]

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "append-blocks",
  "parameters": { "block_id": "example_block_id", "blocks": "example_blocks" }
}
```

---

#### append-text

**Description**: Append text content to a page or block. Use the page id (or block_id) from memory context. Creates paragraph blocks with the given text.

**Parameters**:

- **block_id** (string): The page or block ID to append to (use page id from memory context)
- **content** (string): Alias for text — the content to append to the page
- **page_id** (string): Alias for block_id when appending to a page (same as block_id)
- **text** (string) **(required)**: The text to append (required). Pass the exact content to add to the page.

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "append-text",
  "parameters": {
    "block_id": "example_block_id",
    "content": "example_content",
    "page_id": "example_page_id",
    "text": "example_text"
  }
}
```

---

#### create-comment

**Description**: Create a comment on a page or block, or reply to a discussion. Provide either page_id (new comment on page) or discussion_id (reply). Requires Notion integration to have insert comment capability.

**Parameters**:

- **block_id** (string): Block ID to comment on (optional, use instead of page_id)
- **discussion_id** (string): Discussion ID to reply to an existing thread (use instead of page_id)
- **page_id** (string): Page ID to create a comment on (new discussion)
- **text** (string) **(required)**: Comment text content

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "create-comment",
  "parameters": {
    "block_id": "example_block_id",
    "discussion_id": "example_discussion_id",
    "page_id": "example_page_id",
    "text": "example_text"
  }
}
```

---

#### create-database

**Description**: Create a new database in Notion. Specify parent page_id and title. Optionally provide properties schema as JSON.

**Parameters**:

- **parent_page_id** (string) **(required)**: Parent page ID where the database will be created
- **properties** (string): JSON string of properties schema. Example: {"Name":{"title":{}},"Status":{"select":{"options":[{"name":"Todo"},{"name":"Done"}]}}}
- **title** (string) **(required)**: Database title

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "create-database",
  "parameters": {
    "parent_page_id": "example_parent_page_id",
    "properties": "example_properties",
    "title": "example_title"
  }
}
```

---

#### create-page

**Description**: Create a new page in Notion. Parent can be another page or a database. For database parents, properties must match the database schema.

**Parameters**:

- **content** (string): Initial text content (creates a paragraph block)
- **parent_id** (string) **(required)**: Parent page ID or database ID
- **parent_type** (string): Type of parent (default: page_id)
- **properties** (string): JSON string of additional properties (for database pages)
- **title** (string) **(required)**: Page title

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "create-page",
  "parameters": {
    "content": "example_content",
    "parent_id": "example_parent_id",
    "parent_type": "example_parent_type",
    "properties": "example_properties",
    "title": "example_title"
  }
}
```

---

#### delete-block

**Description**: Delete a block. Permanently removes the block from Notion.

**Parameters**:

- **block_id** (string) **(required)**: The block ID to delete

**Usage Context**: Available in all environments

**Example**:

```json
{ "tool": "delete-block", "parameters": { "block_id": "example_block_id" } }
```

---

#### delete-page

**Description**: Delete (archive) a page. Archived pages can be restored from Notion's trash.

**Parameters**:

- **page_id** (string) **(required)**: The page ID to delete/archive

**Usage Context**: Available in all environments

**Example**:

```json
{ "tool": "delete-page", "parameters": { "page_id": "example_page_id" } }
```

---

#### summarize-pages

**Description**: AI summarization of Notion pages is now handled by the backend server. Synced page content is submitted to the server which runs summarization.

**Parameters**: _None_

**Usage Context**: Available in all environments

**Example**:

```json
{ "tool": "summarize-pages", "parameters": {} }
```

---

#### sync-now

**Description**: Trigger an immediate Notion sync to refresh local data. Returns sync results including counts of synced pages and databases.

**Parameters**: _None_

**Usage Context**: Available in all environments

**Example**:

```json
{ "tool": "sync-now", "parameters": {} }
```

---

#### update-block

**Description**: Update a block's content. The structure depends on the block type.

**Parameters**:

- **archived** (string): Set to true to archive the block
- **block_id** (string) **(required)**: The block ID to update
- **content** (string): JSON string of the block type content. Example for paragraph: {"paragraph":{"rich_text":[{"text":{"content":"Updated text"}}]}}

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "update-block",
  "parameters": {
    "archived": "example_archived",
    "block_id": "example_block_id",
    "content": "example_content"
  }
}
```

---

#### update-database

**Description**: Update a database's title or properties schema.

**Parameters**:

- **database_id** (string) **(required)**: The database ID to update
- **properties** (string): JSON string of properties to add or update
- **title** (string): New title (optional)

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "update-database",
  "parameters": {
    "database_id": "example_database_id",
    "properties": "example_properties",
    "title": "example_title"
  }
}
```

---

#### update-page

**Description**: Update a page's properties. Can update title and other properties. Use append-text to add content blocks.

**Parameters**:

- **archived** (string): Set to true to archive the page
- **page_id** (string) **(required)**: The page ID to update
- **properties** (string): JSON string of properties to update
- **title** (string): New title (optional)

**Usage Context**: Available in all environments

**Example**:

```json
{
  "tool": "update-page",
  "parameters": {
    "archived": "example_archived",
    "page_id": "example_page_id",
    "properties": "example_properties",
    "title": "example_title"
  }
}
```

---

## Tool Usage Guidelines

### Authentication

- All tools require proper authentication setup through the Skills system
- OAuth credentials are managed securely and refreshed automatically
- API keys are stored encrypted in the application keychain

### Rate Limiting

- Tools automatically respect API rate limits of external services
- Intelligent retry logic handles temporary failures with exponential backoff

### Error Handling

- All tools return structured error responses with detailed information
- Network failures trigger automatic retry with configurable attempts

---

**Tool Statistics**

- Total Tools: 12
- Active Skills: 1
- Read/Query Tools: 0 (handled by memory layer)
- Last Updated: 2026-03-26

_This file was automatically generated when the app loaded._
_Tools are discovered from the running V8 skills runtime._
