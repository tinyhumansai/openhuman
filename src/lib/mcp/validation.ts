/**
 * Validation utilities for MCP tools
 */

export class ValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ValidationError';
  }
}

/**
 * Validate chat_id or user_id parameter
 * Supports integer IDs, string IDs, and usernames
 */
export function validateId(value: unknown, paramName: string): number | string {
  if (typeof value === 'number') {
    if (!Number.isInteger(value) || value < -(2 ** 63) || value > 2 ** 63 - 1) {
      throw new ValidationError(
        `Invalid ${paramName}: ${value}. ID is out of the valid integer range.`
      );
    }
    return value;
  }

  if (typeof value === 'string') {
    const intValue = Number.parseInt(value, 10);
    if (!Number.isNaN(intValue) && Number.isFinite(intValue)) {
      if (intValue < -(2 ** 63) || intValue > 2 ** 63 - 1) {
        throw new ValidationError(
          `Invalid ${paramName}: ${value}. ID is out of the valid integer range.`
        );
      }
      return intValue;
    }

    if (/^@?[a-zA-Z0-9_]{5,}$/.test(value)) {
      return value.startsWith('@') ? value : `@${value}`;
    }

    throw new ValidationError(
      `Invalid ${paramName}: '${value}'. Must be a valid integer ID or a username string.`
    );
  }

  throw new ValidationError(
    `Invalid ${paramName}: ${String(value)}. Type must be an integer or a string.`
  );
}

/**
 * Validate list of IDs
 */
export function validateIdList(value: unknown, paramName: string): Array<number | string> {
  if (!Array.isArray(value)) {
    throw new ValidationError(`Invalid ${paramName}: must be an array of IDs.`);
  }

  return value.map((item: unknown, index: number) => {
    try {
      return validateId(item, `${paramName}[${index}]`);
    } catch (error) {
      if (error instanceof ValidationError) {
        throw error;
      }
      const errorMsg = error instanceof Error ? error.message : String(error);
      throw new ValidationError(`Invalid ${paramName}[${index}]: ${errorMsg}`);
    }
  });
}

/**
 * Validate a positive integer parameter (e.g. message IDs)
 */
export function validatePositiveInt(value: unknown, paramName: string): number {
  if (typeof value === 'number') {
    if (!Number.isInteger(value) || value <= 0) {
      throw new ValidationError(`Invalid ${paramName}: ${value}. Must be a positive integer.`);
    }
    return value;
  }

  if (typeof value === 'string') {
    const intValue = Number.parseInt(value, 10);
    if (Number.isNaN(intValue) || intValue <= 0) {
      throw new ValidationError(`Invalid ${paramName}: '${value}'. Must be a positive integer.`);
    }
    return intValue;
  }

  throw new ValidationError(`Invalid ${paramName}: ${String(value)}. Must be a positive integer.`);
}

/**
 * Validate optional ID (can be undefined)
 */
export function validateOptionalId(value: unknown, paramName: string): number | string | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  return validateId(value, paramName);
}
