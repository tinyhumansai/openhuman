import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

export interface GmailEmailEntity {
  identifier: string;
  kind: 'sender' | 'recipient' | 'recipient_cc' | string;
  name: string;
}

export interface GmailEmailMetadata {
  emailId: string;
  threadId: string;
  date: number;
}

export interface GmailEmailChunk {
  content: string;
  entities: GmailEmailEntity[];
  labels: string[];
  metadata: GmailEmailMetadata;
  title: string;
}

export interface GmailEmailBatch {
  chunks: GmailEmailChunk[]; // up to 20 in your example
  createdAt: number; // when this batch was generated
  emailIds: string[]; // same ids as in chunks[*].emailId
  total: number; // total emails in this batch
}

export interface GmailProfile {
  email_address: string;
  messages_total: number;
  threads_total: number;
  history_id: string;
}

interface GmailState {
  /** Emails fetched after OAuth connection (from Gmail skill) */
  emails: GmailEmailBatch | null;
  /** Profile of the connected Gmail user (from Gmail skill) */
  profile: GmailProfile | null;
}

const initialState: GmailState = { emails: null, profile: null };

const gmailSlice = createSlice({
  name: 'gmail',
  initialState,
  reducers: {
    setGmailEmails(state, action: PayloadAction<GmailEmailBatch | null>) {
      state.emails = action.payload;
    },
    clearGmailEmails(state) {
      state.emails = null;
    },
    setGmailProfile(state, action: PayloadAction<GmailProfile | null>) {
      state.profile = action.payload;
    },
    clearGmailProfile(state) {
      state.profile = null;
    },
  },
});

export const { setGmailEmails, clearGmailEmails, setGmailProfile, clearGmailProfile } =
  gmailSlice.actions;
export default gmailSlice.reducer;
