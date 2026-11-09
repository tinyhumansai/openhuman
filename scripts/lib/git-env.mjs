// Repository-local variables exported by Git hooks override a child process's cwd.
// Foreign-repository commands must discover their own checkout instead.
const LOCAL_GIT_VARIABLES = new Set([
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_CONFIG",
  "GIT_CONFIG_PARAMETERS",
  "GIT_CONFIG_COUNT",
  "GIT_OBJECT_DIRECTORY",
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_IMPLICIT_WORK_TREE",
  "GIT_GRAFT_FILE",
  "GIT_INDEX_FILE",
  "GIT_NO_REPLACE_OBJECTS",
  "GIT_REPLACE_REF_BASE",
  "GIT_PREFIX",
  "GIT_SHALLOW_FILE",
  "GIT_COMMON_DIR",
]);

/** Preserve the process environment without Git's parent-repository bindings. */
export function repositoryGitEnv(environment = process.env) {
  return Object.fromEntries(
    Object.entries(environment).filter(
      ([name]) =>
        !LOCAL_GIT_VARIABLES.has(name) &&
        !/^GIT_CONFIG_(?:KEY|VALUE)_\d+$/.test(name),
    ),
  );
}
