/**
 * Fixture-based self-tests for check-storage-owner.mjs
 *
 * Run with: node --test scripts/__tests__/check-storage-owner.test.mjs
 *
 * Cargo metadata dep_kinds format:
 *   - Normal dep: { kind: "normal" } or absent (treated as normal by cargo)
 *   - Build dep:  { kind: "build" }
 *   - Dev dep:    { kind: "dev" }
 */

import { ENGINE_EXCEPTIONS, checkMetadata, formatReport, staleExceptions } from '../ci/check-storage-owner.mjs';

const ROOT = '/fake/repo';
const TSD_MANIFEST = `${ROOT}/vendor/tinystoragedrivers/crates/tinystoragedrivers-sqlite/Cargo.toml`;
const REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index';
const tsd_git = (tag) => `git+https://github.com/tinyhumansai/tinystoragedrivers?tag=${tag}#`;

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

/** Path crate (first-party, source = null). */
const pathPkg = (name, version, manifest, deps = []) => ({
  id: `${name} ${version}`,
  name,
  version,
  manifest_path: manifest,
  source: null,
  dependencies: deps,
});

/** Registry crate. */
const regPkg = (name, version, manifest, deps = []) => ({
  id: `${name} ${version} ${REGISTRY}`,
  name,
  version,
  manifest_path: manifest,
  source: REGISTRY,
  dependencies: deps,
});

/** Normal dep_kinds entry: `cargo metadata` reports a normal edge as `kind: null`. */
const NORMAL = [{ kind: null }];

/** Resolve node for a path crate. */
const pathNode = (name, version, deps) => ({
  id: `${name} ${version}`,
  deps: deps.map(([dep_name, dep_ver, dep_source]) => ({
    name: dep_name,
    pkg: dep_source ? `${dep_name} ${dep_ver} ${dep_source}` : `${dep_name} ${dep_ver}`,
    dep_kinds: NORMAL,
  })),
});

/** Resolve node for a registry crate. */
const regNode = (name, version, deps) => ({
  id: `${name} ${version} ${REGISTRY}`,
  deps: deps.map(([dep_name, dep_ver, dep_source]) => ({
    name: dep_name,
    pkg: dep_source ? `${dep_name} ${dep_ver} ${dep_source}` : `${dep_name} ${dep_ver}`,
    dep_kinds: NORMAL,
  })),
});

// ---------------------------------------------------------------------------
// Fixture: transitive engine through a non-engine tsd crate → must FAIL
// tinyagents-session (path) -> tinystoragedrivers-core (path) -> rusqlite (registry)
// ---------------------------------------------------------------------------

const TSD_CORE_MANIFEST = `${ROOT}/vendor/tinystoragedrivers/crates/tinystoragedrivers-core/Cargo.toml`;

const transitive_engine_fixture = {
  packages: [
    pathPkg('tinystoragedrivers-core', '1.0.0', TSD_CORE_MANIFEST, [
      { name: 'rusqlite', pkg: `rusqlite 0.32.1 ${REGISTRY}` },
    ]),
    regPkg('rusqlite', '0.32.1', `${ROOT}/.cargo/registry/src/foo/rusqlite-0.32.1/Cargo.toml`),
    pathPkg('tinyagents-session', '0.1.0', `${ROOT}/crates/tinyagents-session/Cargo.toml`, [
      { name: 'tinystoragedrivers-core', pkg: 'tinystoragedrivers-core 1.0.0' },
    ]),
  ],
  resolve: {
    nodes: [
      pathNode('tinyagents-session', '0.1.0', [['tinystoragedrivers-core', '1.0.0', null]]),
      pathNode('tinystoragedrivers-core', '1.0.0', [['rusqlite', '0.32.1', REGISTRY]]),
      regNode('rusqlite', '0.32.1', []),
    ],
  },
};

// ---------------------------------------------------------------------------
// Fixture: tsd tag mismatch → must FAIL
// some-odd-crate (path) depends on tinystoragedrivers-core with git tag "99.0.0"
// but vendored version is "1.0.0".
// Note: dep.source must be set for the mismatch check to read the tag.
// ---------------------------------------------------------------------------

const tsd_tag_mismatch_fixture = {
  packages: [
    pathPkg('tinystoragedrivers-core', '1.0.0',
      `${ROOT}/vendor/tinystoragedrivers/crates/tinystoragedrivers-core/Cargo.toml`),
    // some-odd-crate depends on tinystoragedrivers-core with an explicit git source
    // whose tag differs from the vendored version.
    {
      id: `some-odd-crate 0.1.0`,
      name: 'some-odd-crate',
      version: '0.1.0',
      manifest_path: `${ROOT}/crates/some-odd-crate/Cargo.toml`,
      source: null,
      dependencies: [
        {
          name: 'tinystoragedrivers-core',
          pkg: `tinystoragedrivers-core 1.0.0 ${tsd_git('99.0.0')}`,
          source: tsd_git('99.0.0'),  // <-- must be set for tag mismatch check
        },
      ],
    },
  ],
  resolve: {
    nodes: [
      pathNode('some-odd-crate', '0.1.0', [['tinystoragedrivers-core', '1.0.0', tsd_git('99.0.0')]]),
      pathNode('tinystoragedrivers-core', '1.0.0', []),
    ],
  },
};

// ---------------------------------------------------------------------------
// Fixture: allowlisted root reaches engine → must PASS
// openhuman (allowlisted) -> rusqlite
// ---------------------------------------------------------------------------

const allowlisted_root_fixture = {
  packages: [
    pathPkg('openhuman', '0.1.0', `${ROOT}/crates/openhuman/Cargo.toml`, [
      { name: 'rusqlite', pkg: `rusqlite 0.32.1 ${REGISTRY}` },
    ]),
    regPkg('rusqlite', '0.32.1', '/fake/.cargo/registry/src/foo/rusqlite-0.32.1/Cargo.toml'),
  ],
  resolve: {
    nodes: [
      pathNode('openhuman', '0.1.0', [['rusqlite', '0.32.1', REGISTRY]]),
      regNode('rusqlite', '0.32.1', []),
    ],
  },
};

// ---------------------------------------------------------------------------
// Fixture: crate naming violation → must FAIL
// Name ends with "-sqlite" → forbidden pattern.
// ---------------------------------------------------------------------------

const naming_violation_fixture = {
  packages: [
    pathPkg('tinyagents-sqlite', '0.1.0', `${ROOT}/crates/tinyagents-sqlite/Cargo.toml`),
  ],
  resolve: {
    nodes: [
      pathNode('tinyagents-sqlite', '0.1.0', []),
    ],
  },
};

// ---------------------------------------------------------------------------
// Fixture: all tsd crates vendored → must PASS
// ---------------------------------------------------------------------------

const vendored_ok_fixture = {
  packages: [
    pathPkg('tinystoragedrivers-core', '1.0.0',
      `${ROOT}/vendor/tinystoragedrivers/crates/tinystoragedrivers-core/Cargo.toml`),
    pathPkg('tinystoragedrivers-sqlite', '1.0.0', TSD_MANIFEST),
    pathPkg('openhuman-embed', '0.1.0', `${ROOT}/crates/openhuman-embed/Cargo.toml`, [
      { name: 'tinystoragedrivers-core', pkg: 'tinystoragedrivers-core 1.0.0' },
    ]),
  ],
  resolve: {
    nodes: [
      pathNode('tinystoragedrivers-core', '1.0.0', []),
      pathNode('tinystoragedrivers-sqlite', '1.0.0', []),
      pathNode('openhuman-embed', '0.1.0', [['tinystoragedrivers-core', '1.0.0', null]]),
    ],
  },
};

// ---------------------------------------------------------------------------
// Fixture: aws-sdk-s3 reached directly from non-allowlisted crate → must FAIL
// openhuman-core (NOT allowlisted) -> aws-sdk-s3 (registry)
// ---------------------------------------------------------------------------

const aws_s3_fixture = {
  packages: [
    pathPkg('openhuman-core', '0.1.0', `${ROOT}/crates/openhuman-core/Cargo.toml`, [
      { name: 'aws-sdk-s3', pkg: `aws-sdk-s3 1.0.0 ${REGISTRY}` },
    ]),
    regPkg('aws-sdk-s3', '1.0.0', '/fake/.cargo/registry/src/aws/aws-sdk-s3-1.0.0/Cargo.toml'),
  ],
  resolve: {
    nodes: [
      pathNode('openhuman-core', '0.1.0', [['aws-sdk-s3', '1.0.0', REGISTRY]]),
      regNode('aws-sdk-s3', '1.0.0', []),
    ],
  },
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const { test, describe } = await import('node:test');

describe('transitive engine through a non-engine tsd crate → must FAIL', () => {
  test('reports rusqlite and the path that reaches it', () => {
    const violations = checkMetadata(transitive_engine_fixture, ROOT);
    const engineViolations = violations.filter(v => v.kind === 'engine-reach');
    if (
      engineViolations.length !== 1 ||
      engineViolations[0].engine !== 'rusqlite' ||
      engineViolations[0].path.join(' -> ') !== 'tinyagents-session -> tinystoragedrivers-core -> rusqlite'
    ) {
      throw new Error(`Expected one rusqlite engine-reach via tinystoragedrivers-core, got: ${JSON.stringify(violations)}`);
    }
  });
});

describe('tsd tag mismatch → must FAIL', () => {
  test('produces a tsd-tag-mismatch violation', () => {
    const violations = checkMetadata(tsd_tag_mismatch_fixture, ROOT);
    const mismatch = violations.filter(v => v.kind === 'tsd-tag-mismatch');
    if (mismatch.length === 0) {
      throw new Error(`Expected tsd-tag-mismatch violation, got: ${JSON.stringify(violations)}`);
    }
  });
});

describe('allowlisted root reaches engine → must PASS', () => {
  test('produces no violations', () => {
    const violations = checkMetadata(allowlisted_root_fixture, ROOT);
    if (violations.length > 0) {
      throw new Error(`Expected no violations for allowlisted root, got: ${JSON.stringify(violations)}`);
    }
  });
});

describe('crate naming violation → must FAIL', () => {
  test('produces a crate-name violation', () => {
    const violations = checkMetadata(naming_violation_fixture, ROOT);
    const naming = violations.filter(v => v.kind === 'crate-name');
    if (naming.length === 0) {
      throw new Error(`Expected crate-name violation, got: ${JSON.stringify(violations)}`);
    }
  });
});

describe('all tsd crates vendored → must PASS', () => {
  test('produces no violations', () => {
    const violations = checkMetadata(vendored_ok_fixture, ROOT);
    if (violations.length > 0) {
      throw new Error(`Expected no violations for vendored-clean tree, got: ${JSON.stringify(violations)}`);
    }
  });
});

describe('aws-sdk-s3 reached directly → must FAIL', () => {
  test('produces an engine-reach violation for aws-sdk-s3', () => {
    const violations = checkMetadata(aws_s3_fixture, ROOT);
    const engineViolations = violations.filter(v =>
      v.kind === 'engine-reach' && v.engine === 'aws-sdk-s3'
    );
    if (engineViolations.length === 0) {
      throw new Error(`Expected engine-reach violation for aws-sdk-s3, got: ${JSON.stringify(violations)}`);
    }
  });
});

// ---------------------------------------------------------------------------
// Edge kinds, report text, root anchoring and the CLI
// ---------------------------------------------------------------------------

const assert = (await import('node:assert/strict')).default;
const { spawnSync } = await import('node:child_process');
const { mkdtempSync, rmSync, writeFileSync } = await import('node:fs');
const { tmpdir } = await import('node:os');
const { dirname, join, resolve } = await import('node:path');
const { fileURLToPath } = await import('node:url');
const CHECKER = resolve(dirname(fileURLToPath(import.meta.url)), '../ci/check-storage-owner.mjs');

describe('a dev-only edge to an engine → must PASS', () => {
  test('dev dependencies are not part of the closure', () => {
    const fixture = {
      packages: [
        pathPkg('tinyflows', '0.1.0', `${ROOT}/vendor/tinyflows/Cargo.toml`, [
          { name: 'rusqlite', pkg: `rusqlite 0.32.1 ${REGISTRY}` },
        ]),
        regPkg('rusqlite', '0.32.1', '/fake/.cargo/registry/src/foo/rusqlite-0.32.1/Cargo.toml'),
      ],
      resolve: {
        nodes: [
          {
            id: 'tinyflows 0.1.0',
            deps: [{ name: 'rusqlite', pkg: `rusqlite 0.32.1 ${REGISTRY}`, dep_kinds: [{ kind: 'dev' }] }],
          },
          regNode('rusqlite', '0.32.1', []),
        ],
      },
    };
    assert.deepEqual(checkMetadata(fixture, ROOT), []);
  });
});

describe('the report', () => {
  test('names the offender and the engine', () => {
    assert.match(formatReport(checkMetadata(aws_s3_fixture, ROOT)), /openhuman-core: reaches aws-sdk-s3/);
    assert.match(formatReport([]), /ok/);
  });
});

describe('the vendored tinystoragedrivers path is anchored to the repository root', () => {
  test('only <root>/vendor/tinystoragedrivers/ counts as vendored', () => {
    const tsdAt = manifest => ({
      packages: [pathPkg('tinystoragedrivers-core', '1.0.0', manifest)],
      resolve: { nodes: [pathNode('tinystoragedrivers-core', '1.0.0', [])] },
    });
    assert.deepEqual(checkMetadata(tsdAt(`${ROOT}/vendor/tinystoragedrivers/crates/core/Cargo.toml`), ROOT), []);
    for (const bad of [
      '/other/vendor/tinystoragedrivers/crates/core/Cargo.toml',
      `${ROOT}/vendor/x/vendor/tinystoragedrivers/crates/core/Cargo.toml`,
    ]) {
      assert.equal(checkMetadata(tsdAt(bad), ROOT)[0]?.kind, 'tsd-not-vendored', bad);
    }
  });
});

describe('the CLI', () => {
  test('exits 1 on a violation, 0 when clean, 2 on unreadable input', () => {
    const dir = mkdtempSync(join(tmpdir(), 'storage-owner-'));
    try {
      const bad = join(dir, 'bad.json');
      const good = join(dir, 'good.json');
      writeFileSync(bad, JSON.stringify(aws_s3_fixture));
      writeFileSync(good, JSON.stringify(allowlisted_root_fixture));
      assert.equal(spawnSync('node', [CHECKER, '--metadata', bad]).status, 1);
      assert.equal(spawnSync('node', [CHECKER, '--metadata', good]).status, 0);
      assert.equal(spawnSync('node', [CHECKER, '--metadata', join(dir, 'missing.json')]).status, 2);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

// ---------------------------------------------------------------------------
// Normal edges, composition roots and engine exceptions
// ---------------------------------------------------------------------------

/** A one-path chain fixture: names[0] -> names[1] -> ... ; the last is a registry crate. */
const chain = (...names) => {
  const last = names.length - 1;
  const id = (name, i) => (i === last ? `${name} 1.0.0 ${REGISTRY}` : `${name} 1.0.0`);
  return {
    packages: names.map((name, i) =>
      i === last
        ? regPkg(name, '1.0.0', `/fake/.cargo/registry/src/x/${name}-1.0.0/Cargo.toml`)
        : pathPkg(name, '1.0.0', `${ROOT}/vendor/${name}/Cargo.toml`, [
            { name: names[i + 1], pkg: id(names[i + 1], i + 1) },
          ])
    ),
    resolve: {
      nodes: names.map((name, i) => ({
        id: id(name, i),
        deps: i === last ? [] : [{ name: names[i + 1], pkg: id(names[i + 1], i + 1), dep_kinds: NORMAL }],
      })),
    },
  };
};

describe('a normal edge, as cargo reports it (kind: null)', () => {
  test('is followed to the engine', () => {
    const v = checkMetadata(chain('tinyflows', 'rusqlite'), ROOT);
    assert.equal(v.length, 1);
    assert.equal(v[0].engine, 'rusqlite');
  });
});

describe('a composition root', () => {
  test('stops the walk: what the root reaches is its own', () => {
    assert.deepEqual(checkMetadata(chain('openhuman-embed', 'openhuman', 'rusqlite'), ROOT), []);
  });
});

describe('ENGINE_EXCEPTIONS', () => {
  test('excuse exactly their edge, wherever the walk starts, and record the match', () => {
    const used = new Set();
    assert.deepEqual(checkMetadata(chain('tinyagents-harness', 'tinyagents-harness-tools', 'fs2'), ROOT, used), []);
    assert.deepEqual([...used], ['tinyagents-harness-tools -> fs2']);
  });

  test('do not hide another edge into the same engine', () => {
    // tinyflows -> tinyagents-harness-tools -> fs2 (excepted) and tinyflows -> lockutil -> fs2 (not).
    const reg = name => `${name} 1.0.0 ${REGISTRY}`;
    const fixture = {
      packages: [
        pathPkg('tinyflows', '1.0.0', `${ROOT}/vendor/tinyflows/Cargo.toml`),
        pathPkg('tinyagents-harness-tools', '1.0.0', `${ROOT}/vendor/tinyagents-harness-tools/Cargo.toml`),
        regPkg('lockutil', '1.0.0', '/fake/.cargo/registry/src/x/lockutil-1.0.0/Cargo.toml'),
        regPkg('fs2', '1.0.0', '/fake/.cargo/registry/src/x/fs2-1.0.0/Cargo.toml'),
      ],
      resolve: {
        nodes: [
          {
            id: 'tinyflows 1.0.0',
            deps: [
              { name: 'tinyagents-harness-tools', pkg: 'tinyagents-harness-tools 1.0.0', dep_kinds: NORMAL },
              { name: 'lockutil', pkg: reg('lockutil'), dep_kinds: NORMAL },
            ],
          },
          { id: 'tinyagents-harness-tools 1.0.0', deps: [{ name: 'fs2', pkg: reg('fs2'), dep_kinds: NORMAL }] },
          { id: reg('lockutil'), deps: [{ name: 'fs2', pkg: reg('fs2'), dep_kinds: NORMAL }] },
          { id: reg('fs2'), deps: [] },
        ],
      },
    };
    const v = checkMetadata(fixture, ROOT).filter(x => x.package === 'tinyflows');
    assert.deepEqual(
      v.map(x => x.path.join(' -> ')),
      ['tinyflows -> lockutil -> fs2']
    );
  });

  test('do not excuse another engine or another package', () => {
    assert.equal(checkMetadata(chain('tinyagents-harness-tools', 'rusqlite'), ROOT)[0]?.engine, 'rusqlite');
    assert.equal(checkMetadata(chain('tinytools-core', 'fs2'), ROOT)[0]?.engine, 'fs2');
  });

  test('every entry names its edge, a reason and a follow-up', () => {
    for (const e of ENGINE_EXCEPTIONS) {
      assert.ok(e.package && e.engine && e.reason.length > 10 && e.followUp.length > 10, JSON.stringify(e));
    }
  });

  test('an entry nothing matched is reported as stale', () => {
    assert.equal(staleExceptions(new Set()).length, ENGINE_EXCEPTIONS.length);
    assert.equal(staleExceptions(new Set()).every(v => v.kind === 'stale-exception'), true);
    const all = new Set(ENGINE_EXCEPTIONS.map(e => `${e.package} -> ${e.engine}`));
    assert.deepEqual(staleExceptions(all), []);
  });
});
