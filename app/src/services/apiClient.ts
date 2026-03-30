import type { ApiError } from '../types/api';
import { getBackendUrl } from './backendUrl';

type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

interface RequestOptions {
  method?: HttpMethod;
  body?: unknown;
  headers?: Record<string, string>;
  requireAuth?: boolean;
  timeout?: number;
}

/**
 * Lazy auth token accessor so `apiClient` never imports `store/index` at module level.
 * Entry (`main.tsx`) and Vitest setup call `setStoreForApiClient` after the store module loads,
 * avoiding a cycle: `store` → `apiClient` → … → `socketService` → `store`.
 *
 * The binding name avoids clashing with transpiled private method names (e.g. `_getToken`).
 */
let authTokenGetterRef: (() => string | null) | null = null;

export function setStoreForApiClient(getToken: () => string | null) {
  authTokenGetterRef = getToken;
}

/**
 * API Client for making requests to the backend
 * Handles authentication, error handling, and response typing
 */
class ApiClient {
  private resolveAuthToken(): string | null {
    return authTokenGetterRef ? authTokenGetterRef() : null;
  }

  /**
   * Build headers for the request
   */
  private buildHeaders(options: RequestOptions): HeadersInit {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...options.headers,
    };

    // Add authorization header if auth is required
    if (options.requireAuth !== false) {
      const token = this.resolveAuthToken();
      if (token) {
        headers.Authorization = `Bearer ${token}`;
      }
    }

    return headers;
  }

  /**
   * Make an API request
   */
  private async request<T>(endpoint: string, options: RequestOptions = {}): Promise<T> {
    const { method = 'GET', body, requireAuth = true, timeout = 120_000 } = options;

    const baseUrl = await getBackendUrl();
    const url = `${baseUrl}${endpoint}`;
    const headers = this.buildHeaders({ ...options, requireAuth });

    console.log('request', { url, headers, body, method });

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), timeout);

    const config: RequestInit = { method, headers, signal: controller.signal };

    if (body && method !== 'GET') {
      config.body = JSON.stringify(body);
    }

    try {
      const response = await fetch(url, config);

      // Handle non-JSON responses
      const contentType = response.headers.get('content-type');
      if (!contentType || !contentType.includes('application/json')) {
        if (!response.ok) {
          throw new Error(`HTTP error! status: ${response.status}`);
        }
        return {} as T;
      }

      const data = await response.json();

      // Handle error responses
      if (!response.ok) {
        const error: ApiError = data.error
          ? { success: false, error: data.error, message: data.message }
          : { success: false, error: `HTTP ${response.status}: ${response.statusText}` };
        throw error;
      }

      return data as T;
    } catch (error) {
      // Re-throw API errors as-is
      if (error && typeof error === 'object' && 'error' in error) {
        throw error;
      }

      // Handle abort/timeout specifically
      if (error instanceof DOMException && error.name === 'AbortError') {
        throw { success: false, error: `Request timed out after ${timeout / 1000}s` } as ApiError;
      }

      // Wrap network/other errors
      throw {
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error occurred',
      } as ApiError;
    } finally {
      clearTimeout(timeoutId);
    }
  }

  /**
   * GET request
   */
  async get<T>(endpoint: string, options?: Omit<RequestOptions, 'method' | 'body'>): Promise<T> {
    return this.request<T>(endpoint, { ...options, method: 'GET' });
  }

  /**
   * POST request
   */
  async post<T>(
    endpoint: string,
    body?: unknown,
    options?: Omit<RequestOptions, 'method' | 'body'>
  ): Promise<T> {
    return this.request<T>(endpoint, { ...options, method: 'POST', body });
  }

  /**
   * PUT request
   */
  async put<T>(
    endpoint: string,
    body?: unknown,
    options?: Omit<RequestOptions, 'method' | 'body'>
  ): Promise<T> {
    return this.request<T>(endpoint, { ...options, method: 'PUT', body });
  }

  /**
   * PATCH request
   */
  async patch<T>(
    endpoint: string,
    body?: unknown,
    options?: Omit<RequestOptions, 'method' | 'body'>
  ): Promise<T> {
    return this.request<T>(endpoint, { ...options, method: 'PATCH', body });
  }

  /**
   * DELETE request
   */
  async delete<T>(endpoint: string, options?: Omit<RequestOptions, 'method' | 'body'>): Promise<T> {
    return this.request<T>(endpoint, { ...options, method: 'DELETE' });
  }
}

// Export singleton instance
export const apiClient = new ApiClient();
