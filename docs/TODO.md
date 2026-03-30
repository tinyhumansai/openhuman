todo

- allow skills to be downloaded from the web
- allow skills to be written as text formatted files like SKILL.md
- skills need to specific via JSON-rpc the state changes they make to their state and data files in memory
- skills need to be able to download custom mcp servers from the web

- integrate the payments flow properly, skip the connect account page and goto the home page

[] - allow for new skills to be coded on their own
[] - allow for multiple instances of a skill to be loaded
[] - add a local model that can read through the screen and also go through voice using an API like whisper
[] - add a screener recorder that goes through the intefaces in the screen and locally summarizes what is happening and brings more assitance to the user
[] clean up the core so that we can run it as a binary on a server or as docker

[x] Separate the binary from the tauri codebase
[] Integrate our custom memory engine into core
[] Integrate our skills registry into core
[x] Integrate accessibility service installation
[] Add as a step and setting in the UI
[x] Remove mentions of zeroclaw from the codebaes
[x] Integrate local LLM into core
[] Handle process/deamon properly
[x] install the linux philosophy of few modules that do their own thing really well sort of..
[x] Remove android / ios support from the codebase.
[] e2e test to check if daemon and sidecar loading works properly
[x] Find a better way to structure the cargo files
[x] fix all the rust and cargo issues
[] Add icon and app name to the various permission settings
[] add self update based on github release. create a update action on the cli
[] for each skill show information on how much data has been synced locally and information on how much syncs have happened so far etc..
[x] redo the docs once everything is done.
[x] remove unwanted feature flags from the rust binary
[] fix the config properly
[] Allow for Migrating from OpenClaw
[] allow users to choose which version of LLM model they'd like to choose based on their CPU. better ram and gpu means higher parameter model can be used.
[] in the client side app, make console.log follow a logger style logging where there's a namespace for every logger (like python)
