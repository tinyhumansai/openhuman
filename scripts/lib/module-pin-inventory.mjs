// Shared source inventory for the module-pin gate and its tag-fetch bootstrap.
// This module has no filesystem, subprocess, or process-exit side effects.

// Record id -> the submodule that is the source of truth for its version.
//
// `submodule: null` means "this record has no submodule of its own", and needs a
// reason. It is NOT an exemption from drift. Runtime providers are
// released from their own repositories on their own version line, so their
// version cannot equal a submodule tag; `provider` instead checks them against
// scripts/ci/module-provider-pins.json (their release, and the commit of
// `builtAgainst` that release was built from, which must be the host's pin).
export const PIN_MAP = {
  tinysecurity: { submodule: "vendor/tinysecurity" },
  tinysearch: { submodule: "vendor/tinysearch" },
  tinycomputer: { submodule: "vendor/tinycomputer" },
  tinybox: { submodule: "vendor/tinybox" },
  tinychannels: { submodule: "vendor/tinychannels" },
  tinyhosts: { submodule: "vendor/tinyhosts" },
  tinydocs: { submodule: "vendor/tinydocs" },
  tinywallet: { submodule: "vendor/tinywallet" },
  tinyjuice: { submodule: "vendor/tinyjuice" },
  tinyvoice: { submodule: "vendor/tinyvoice" },
  tinymcp: { submodule: "vendor/tinymcp" },
  tinyconnectors: { submodule: "vendor/tinyconnectors" },
  tinyflows: { submodule: "vendor/tinyflows" },
  tinyeventqueues: { submodule: "vendor/tinyeventqueues" },
};

/**
 * Return sorted, unique repository paths needed by the pin gate. Providers use
 * builtAgainst; other records use submodule or sharesWith. Records without a
 * configured path are omitted.
 */
export function modulePinRepositories(pinMap = PIN_MAP) {
  return [
    ...new Set(
      Object.values(pinMap)
        .map((entry) =>
          entry.provider
            ? entry.provider.builtAgainst
            : (entry.submodule ?? entry.sharesWith),
        )
        .filter(Boolean),
    ),
  ].sort();
}
