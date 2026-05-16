export type LocalStatus =
  | 'not_started'
  | 'in_progress'
  | 'waiting'
  | 'submitted'
  | 'done'
  | 'unclear';

export type UrgencyLevel = 'critical' | 'high' | 'medium' | 'low' | 'unclear';

export interface CourseMatcher {
  canvas_id?: string | null;
  name: string;
}

export interface CanvasTrackerSettings {
  enabled: boolean;
  host: string;
  allowlisted_courses: CourseMatcher[];
  token_set: boolean;
}

export interface ReminderRecommendation {
  kind: string;
  at?: string | null;
  message: string;
}

export interface CanvasTask {
  course_id: string;
  course_name: string;
  assignment_id: string;
  assignment_name: string;
  due_at?: string | null;
  due_at_unclear: boolean;
  instructions_summary: string;
  submission_type?: string | null;
  canvas_workflow_state?: string | null;
  canvas_submission_state?: string | null;
  local_status: LocalStatus;
  urgency_level: UrgencyLevel;
  recommended_start_at?: string | null;
  reminders_needed: ReminderRecommendation[];
  source_url?: string | null;
  last_seen_at: string;
}

export interface SyncSummary {
  synced: boolean;
  courses_seen: number;
  courses_used: number;
  courses_ignored: number;
  assignments_seen: number;
  tasks_upserted: number;
  previous_tasks_preserved: boolean;
  errors: string[];
  synced_at: string;
}
