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
[x] Integrate our custom memory engine into core - sanil
[x] Integrate our skills registry into core - steve
[x] Integrate accessibility service installation
[] Add as a step and setting in the UI - cyrus
[x] Remove mentions of zeroclaw from the codebaes
[x] Integrate local LLM into core
[x] Handle process/deamon properly
[x] install the linux philosophy of few modules that do their own thing really well sort of..
[x] Remove android / ios support from the codebase.
[x] e2e test to check if daemon and sidecar loading works properly
[x] Find a better way to structure the cargo files
[x] fix all the rust and cargo issues
[] Add icon and app name to the various permission settings - mithil
[] add self update based on github release. create a update action on the cli - aniketh
[x] for each skill show information on how much data has been synced locally and information on how much syncs have happened so far etc.. - mithil/elvin
[x] redo the docs once everything is done.
[x] remove unwanted feature flags from the rust binary
[] fix the config properly - mithil
[] Allow for Migrating from OpenClaw - steve done - to be tested
[] allow users to choose which version of LLM model they'd like to choose based on their CPU. better ram and gpu means higher parameter model can be used. - mithil
[x] in the client side app, make console.log follow a logger style logging where there's a namespace for every logger (like python) - steve
[x] - currently we bundle tauri in the openhumany rust core but that shouldn't really have to be there. it can be completely removed.
[x] allow skills to be debuggged from the UI (we shuold try to call various tools or see state from the UI itself)

[] improve the prompts so that it avoid Hallucination. So that we can start to focus more on useful things.I asked a question on Notion. Instead of identifying that it is not connected and should install Notion, it gave me suggestions on fake Notion pages.

- voiceover functionalities
  [] fix the overlay
  [] get it to listen to meetings
  [] get it to actually use the local whisper model

- screen intelligence

- ollama
  [] fix bug where downloads get iterrupted and it keeps restarting over and over again
  [] fix bug where download progress each download part instead of the whole model (as download happens in parts)
  [] once a model has been downloaded we can hide the model window from the IU

- gmail skill
  [] allow skills to have their oauth setup locally or credentials enterred manually. in which case we will need to ask for oauth creds and setup the webhook urls ourselves.
  [] allows skills to have an index so that we can setup functionality to have multiple instances of skill for a user (mulitple gmail accounts etc etc...).
  [] we need to massively improve the skills development and testing environment so that we can get it as close to production really. so todo that we need to somehow be able to run just the skills runtime from the core rust code within the skills repo so that testing becomes super straightforward (might be heavy, but it'll work)
  [] use encryption to encrypt data back and forth; especially when working with our version of skills
  [] massively simplify the skills flow and codebase (less is better)

- webhook functionality test
  [] create a debug screen to view and test the available webhooks and also monitor their events

- memory skill
  [] should index properly all the things (sanil)

--- e2e tests to write up

- [ ] connecting a channel like telegram/discord works properly
- [] add cmake and tauri driver into the build containers so that we can skip
