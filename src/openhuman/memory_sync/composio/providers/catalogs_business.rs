//! Curated catalogs — business toolkits: Shopify, Stripe, HubSpot,
//! Salesforce, Airtable, Figma.

use super::tool_scope::{CuratedTool, ToolScope};

// ── shopify ─────────────────────────────────────────────────────────
pub const SHOPIFY_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "SHOPIFY_BULK_QUERY_OPERATION",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SHOPIFY_COUNT_PRODUCTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SHOPIFY_COUNT_ORDERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SHOPIFY_COUNT_FULFILLMENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SHOPIFY_COUNT_CUSTOMERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_ORDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_PRODUCT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_DRAFT_ORDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_FULFILLMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_CUSTOMER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_PRICE_RULE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_ADJUST_INVENTORY_LEVEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_DISCOUNT_CODE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_UPDATE_PRODUCT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CREATE_CUSTOM_COLLECTION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SHOPIFY_CANCEL_ORDER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SHOPIFY_CANCEL_FULFILLMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SHOPIFY_DELETE_PRODUCT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SHOPIFY_BULK_DELETE_CUSTOMER_ADDRESSES",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SHOPIFY_BULK_DELETE_METAFIELDS",
        scope: ToolScope::Admin,
    },
];

// ── stripe ──────────────────────────────────────────────────────────
pub const STRIPE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "STRIPE_GET_PAYMENT_INTENT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "STRIPE_LIST_INVOICES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "STRIPE_GET_CUSTOMER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "STRIPE_LIST_CHARGES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "STRIPE_GET_SUBSCRIPTION",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_PAYMENT_INTENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_INVOICE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_CUSTOMER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_CUSTOMER_SUBSCRIPTION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_CHECKOUT_SESSION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CONFIRM_PAYMENT_INTENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CAPTURE_PAYMENT_INTENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_ATTACH_PAYMENT_METHOD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "STRIPE_CANCEL_SUBSCRIPTION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "STRIPE_CANCEL_PAYMENT_INTENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "STRIPE_CREATE_CHARGE_REFUND",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "STRIPE_CLOSE_DISPUTE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "STRIPE_CANCEL_SETUP_INTENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "STRIPE_ARCHIVE_BILLING_ALERT",
        scope: ToolScope::Admin,
    },
];

// ── hubspot ─────────────────────────────────────────────────────────
pub const HUBSPOT_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "HUBSPOT_GET_CONTACTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_SEARCH_CONTACTS_BY_CRITERIA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_LIST_CONTACTS_PAGE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_GET_COMPANIES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_GET_DEALS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_GET_CRM_OBJECT_BY_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_BATCH_READ_COMPANIES_BY_PROPERTIES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_CONTACT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_COMPANY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_DEAL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_CONTACTS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_UPDATE_CONTACT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_UPDATE_COMPANY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_OBJECT_ASSOCIATION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_A_NEW_MARKETING_EMAIL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_CREATE_BATCH_OF_OBJECTS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_BATCH_UPDATE_QUOTES",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_CONTACT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_COMPANY",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_DEAL",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_CONTACTS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_COMPANIES",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_DEALS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_CRM_OBJECT_BY_ID",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "HUBSPOT_ARCHIVE_PROPERTY_BY_OBJECT_TYPE_AND_NAME",
        scope: ToolScope::Admin,
    },
];

// ── salesforce ──────────────────────────────────────────────────────
pub const SALESFORCE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "SALESFORCE_RUN_SOQL_QUERY",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_EXECUTE_SOSL_SEARCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_GET_ACCOUNT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_GET_CAMPAIGN",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_GET_ALL_FIELDS_FOR_OBJECT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_GET_ALL_CUSTOM_OBJECTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_ACCOUNT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_CONTACT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_LEAD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_OPPORTUNITY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_CAMPAIGN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_UPDATE_ACCOUNT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_UPDATE_CONTACT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_UPDATE_OPPORTUNITY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_ADD_OPPORTUNITY_LINE_ITEM",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_ADD_CONTACT_TO_CAMPAIGN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_ADD_LEAD_TO_CAMPAIGN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_ASSOCIATE_CONTACT_TO_ACCOUNT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_CLONE_OPPORTUNITY_WITH_PRODUCTS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_ACCOUNT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_CONTACT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_LEAD",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_OPPORTUNITY",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_CAMPAIGN",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_SOBJECT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_DELETE_SOBJECT_COLLECTIONS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "SALESFORCE_CREATE_CUSTOM_FIELD",
        scope: ToolScope::Admin,
    },
];

// ── airtable ────────────────────────────────────────────────────────
pub const AIRTABLE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "AIRTABLE_LIST_RECORDS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "AIRTABLE_GET_RECORD",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "AIRTABLE_GET_BASE_SCHEMA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "AIRTABLE_LIST_BASES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "AIRTABLE_LIST_COMMENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "AIRTABLE_CREATE_RECORDS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_UPDATE_RECORD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_UPDATE_MULTIPLE_RECORDS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_CREATE_FIELD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_CREATE_TABLE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_CREATE_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_UPLOAD_ATTACHMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_UPDATE_FIELD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_UPDATE_TABLE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "AIRTABLE_DELETE_RECORD",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "AIRTABLE_DELETE_MULTIPLE_RECORDS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "AIRTABLE_DELETE_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "AIRTABLE_CREATE_BASE",
        scope: ToolScope::Admin,
    },
];

// ── figma ───────────────────────────────────────────────────────────
pub const FIGMA_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "FIGMA_GET_FILE_JSON",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_GET_FILE_NODES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_GET_COMMENTS_IN_A_FILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_GET_CURRENT_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_DISCOVER_FIGMA_RESOURCES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_GET_FILE_COMPONENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_GET_LOCAL_VARIABLES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_EXTRACT_DESIGN_TOKENS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "FIGMA_ADD_A_COMMENT_TO_A_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "FIGMA_CREATE_DEV_RESOURCES",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "FIGMA_CREATE_MODIFY_DELETE_VARIABLES",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "FIGMA_DELETE_A_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "FIGMA_DELETE_A_WEBHOOK",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "FIGMA_DELETE_DEV_RESOURCE",
        scope: ToolScope::Admin,
    },
];
