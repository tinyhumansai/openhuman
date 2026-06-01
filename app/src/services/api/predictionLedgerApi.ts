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

function isMissingFileError(err: unknown): boolean {
  if (typeof err === 'object' && err !== null && 'code' in err && err.code === 'ENOENT') {
    return true;
  }
  const message = err instanceof Error ? err.message : String(err);
  return /ENOENT|not found|does not exist/i.test(message);
}

/**
 * Load and parse the ledger.
 *
 * Returns [] for the two BENIGN cases — the file does not exist yet (the read
 * RPC rejects on first run) and an empty/whitespace blob — so a fresh user sees
 * the empty state, not an error.
 *
 * A present-but-unparseable blob is NOT benign: it is real corruption, so this
 * THROWS rather than returning []. Swallowing it would render the ledger as
 * empty and let the next save overwrite the corrupt-but-recoverable file,
 * destroying data. The container surfaces the thrown error instead.
 */
export async function loadLedger(): Promise<PredictionRecord[]> {
  let content: string;
  try {
    content = await aiReadMemoryFile(LEDGER_PATH);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (isMissingFileError(err)) {
      log('[rpc] loadLedger no-ledger-file path=%s reason=%s', LEDGER_PATH, message);
      return [];
    }
    log('[rpc] loadLedger read-failed path=%s reason=%s', LEDGER_PATH, message);
    throw err;
  }
  if (!content || !content.trim()) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch (err) {
    throw new Error(
      `Prediction ledger at ${LEDGER_PATH} is corrupted and could not be parsed: ${
        err instanceof Error ? err.message : String(err)
      }`
    );
  }
  return parsePredictionRecords(parsed);
}

/** Persist the full ledger as pretty-printed JSON (diff-friendly on disk). */
export async function saveLedger(records: PredictionRecord[]): Promise<void> {
  log('[rpc] saveLedger path=%s recordCount=%d', LEDGER_PATH, records.length);
  await aiWriteMemoryFile(LEDGER_PATH, JSON.stringify(records, null, 2));
}

export const predictionLedgerApi = { loadLedger, saveLedger, LEDGER_PATH };
