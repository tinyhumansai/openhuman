import createDebug from 'debug';

import type { ApiResponse } from '../../types/api';
import type {
  CreateFeedbackInput,
  CreateFeedbackResult,
  FeedbackComment,
  FeedbackDetail,
  FeedbackItem,
  FeedbackListParams,
  FeedbackListResult,
  FeedbackStatus,
  FeedbackVoteValue,
} from '../../types/feedback';
import { apiClient } from '../apiClient';

const log = createDebug('feedback:api');

function buildListQuery(params: FeedbackListParams): string {
  const search = new URLSearchParams();
  if (params.type) search.set('type', params.type);
  if (params.status) search.set('status', params.status);
  if (params.sort) search.set('sort', params.sort);
  if (params.page !== undefined) search.set('page', String(params.page));
  if (params.limit !== undefined) search.set('limit', String(params.limit));
  const query = search.toString();
  return query ? `?${query}` : '';
}

export const feedbackApi = {
  /** GET /feedback — paginated public board. */
  listFeedback: async (params: FeedbackListParams = {}): Promise<FeedbackListResult> => {
    const query = buildListQuery(params);
    log('listFeedback params=%o', params);
    const response = await apiClient.get<ApiResponse<FeedbackListResult>>(`/feedback${query}`);
    return response.data;
  },

  /** GET /feedback/:id — a single item with its comments. */
  getFeedback: async (id: string): Promise<FeedbackDetail> => {
    log('getFeedback id=%s', id);
    const response = await apiClient.get<ApiResponse<FeedbackDetail>>(
      `/feedback/${encodeURIComponent(id)}`
    );
    return response.data;
  },

  /** POST /feedback/:id/comments — add a comment (moderated, length-capped). */
  addComment: async (id: string, body: string): Promise<FeedbackComment> => {
    log('addComment id=%s bodyLen=%d', id, body.length);
    const response = await apiClient.post<ApiResponse<FeedbackComment>>(
      `/feedback/${encodeURIComponent(id)}/comments`,
      { body }
    );
    return response.data;
  },

  /** POST /feedback — submit feature/bug feedback (moderated, daily-capped). */
  submitFeedback: async (input: CreateFeedbackInput): Promise<CreateFeedbackResult> => {
    log('submitFeedback type=%s titleLen=%d', input.type, input.title.length);
    const response = await apiClient.post<ApiResponse<CreateFeedbackResult>>('/feedback', input);
    return response.data;
  },

  /** POST /feedback/:id/vote — 1 upvote, -1 downvote, 0 retract. */
  voteFeedback: async (id: string, value: FeedbackVoteValue): Promise<FeedbackItem> => {
    log('voteFeedback id=%s value=%d', id, value);
    const response = await apiClient.post<ApiResponse<FeedbackItem>>(
      `/feedback/${encodeURIComponent(id)}/vote`,
      { value }
    );
    return response.data;
  },

  /** PATCH /feedback/:id/status — admin-only status transition. */
  updateStatus: async (id: string, status: FeedbackStatus): Promise<FeedbackItem> => {
    log('updateStatus id=%s status=%s', id, status);
    const response = await apiClient.patch<ApiResponse<FeedbackItem>>(
      `/feedback/${encodeURIComponent(id)}/status`,
      { status }
    );
    return response.data;
  },
};
