import type { ApiError } from '../types/api';
import { BACKEND_URL } from '../utils/config';

type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

interface RequestOptions {
  method?: HttpMethod;
  body?: unknown;
  headers?: Record<string, string>;
  requireAuth?: boolean;
}

/**
 * Lazy store accessor to break the circular dependency:
 *   store/index → slices → api services → apiClient → store/index
 *
 * The store registers itself via `setStoreForApiClient` after creation,
 * so apiClient never imports store/index at module level.
 */
let _getToken: (() => string | null) | null = null;

export function setStoreForApiClient(getToken: () => string | null) {
  _getToken = getToken;
}

/**
 * API Client for making requests to the backend
 * Handles authentication, error handling, and response typing
 */
class ApiClient {
  private baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl;
  }

  private getToken(): string | null {
    return _getToken ? _getToken() : null;
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
      const token = this.getToken();
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
    const { method = 'GET', body, requireAuth = true } = options;

    const url = `${this.baseUrl}${endpoint}`;
    const headers = this.buildHeaders({ ...options, requireAuth });

    const config: RequestInit = { method, headers };

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

      // Wrap network/other errors
      throw {
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error occurred',
      } as ApiError;
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
export const apiClient = new ApiClient(BACKEND_URL);
