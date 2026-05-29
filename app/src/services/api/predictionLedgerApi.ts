/**
 * Persistence facade for the Prediction Calibration Ledger.
 *
 * Adds ZERO new core surface. The whole ledger is stored as one JSON blob in
 * the workspace memory tree via the already-shipped file RPCs:
 *   - aiReadMemoryFile  (openhuman.memory_read_file)  — load
 *   - aiWriteMemoryFile (openhuman.memory_write_file) — save
 *
 * `memory_doc_ingest` / `memory_list_documents` deliberately ARE NOT used:
 * the list RPC returns only document summaries (no content/metadata), so a
 * prediction's confidence/outcome fields could never be read back. The
 * single-blob file approach round-trips arbitrary JSON deterministically.
 */
import debug from 'debug';

import { parsePredictionRecords, type PredictionRecord } from '../../lib/memory/predictionLedger';
import { aiReadMemoryFile, aiWriteMemoryFile } from '../../utils/tauriCommands/memory';

const log = debug('prediction-ledger:api');

/** Workspace-relative path of the single ledger blob. */
export const LEDGER_PATH = 'predictions/ledger.json';

/**
 * Load and parse the ledger. Returns [] on first run (the read RPC rejects when
 * the file does not exist) and on a malformed/empty blob — never throws — so a
 * fresh user sees the empty state rather than an error.
 */
export async function loadLedger(): Promise<PredictionRecord[]> {
  try {
    const content = await aiReadMemoryFile(LEDGER_PATH);
    if (!content || !content.trim()) return [];
    return parsePredictionRecords(JSON.parse(content));
  } catch (err) {
    log(
      'loadLedger: no readable ledger yet (%s)',
      err instanceof Error ? err.message : String(err)
    );
    return [];
  }
}

/** Persist the full ledger as pretty-printed JSON (diff-friendly on disk). */
export async function saveLedger(records: PredictionRecord[]): Promise<void> {
  log('saveLedger: writing %d records', records.length);
  await aiWriteMemoryFile(LEDGER_PATH, JSON.stringify(records, null, 2));
}

export const predictionLedgerApi = { loadLedger, saveLedger, LEDGER_PATH };
