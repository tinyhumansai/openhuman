# Harness initialization

The harness initialization domain runs startup steps and reports progress to the frontend. Language runtimes are no longer installed during startup; script-backed tools use the host's `node` and `python3` on `PATH`.

The registry currently has no provisioning steps. The remaining status and run endpoints support the frontend initialization surface and future startup work.
