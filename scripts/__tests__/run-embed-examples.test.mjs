import assert from "node:assert/strict";
import { test } from "node:test";
import { assertExampleOutput, discoverExamples, offlineEnvironment, runExamples } from "../run-embed-examples.mjs";

test("the runner strips inherited live credentials and operator workspace settings", () => {
  assert.deepEqual(offlineEnvironment({ PATH: "bin", OPENHUMAN_EXAMPLE_LIVE: "1", OPENHUMAN_EXAMPLE_API_KEY: "secret", OPENHUMAN_BACKEND_API_KEY: "secret", OPENHUMAN_STORAGE_URL: "remote", OPENHUMAN_WORKSPACE: "operator", OPENHUMAN_KEYRING_MASTER_KEY: "operator-key", OPENHUMAN_KEYRING_MASTER_KEY_FILE: "operator-file", OPENHUMAN_KEYRING_BACKEND: "os" }), { PATH: "bin" });
});
test("zero exit without the example's behavioral marker is a failure", () => {
  assert.throws(() => assertExampleOutput("hello", {status:0,stdout:"EXAMPLE_OK another\n",stderr:""}), /behavioral assertion marker/);
  assert.throws(() => assertExampleOutput("hello", {status:1,stdout:"EXAMPLE_OK hello\n",stderr:"failed"}), /exited 1/);
  assert.doesNotThrow(() => assertExampleOutput("hello", {status:0,stdout:"EXAMPLE_OK hello\n",stderr:""}));
});
test("runner executes feature gated examples and checks stdout", () => {
  const calls = [];
  runExamples({examples:[{name:"telegram",features:["channels"]}],environment:{OPENHUMAN_EXAMPLE_LIVE:"1"}, run(command,args,options) {
    calls.push({command,args,options});
    return {status:0,stdout:"EXAMPLE_OK telegram\n",stderr:""};
  }});
  assert.ok(calls[0].command.endsWith("ci-cancel-aware.sh"));
  assert.ok(calls[0].args.includes("--features"));
  assert.ok(calls[0].args.includes("channels"));
  assert.equal(calls[0].options.env.OPENHUMAN_EXAMPLE_LIVE, undefined);
});
test("every discoverable example has an offline output contract", () => {
  const examples = discoverExamples();
  assert.ok(examples.length >= 18);
  assert.ok(examples.some((example) => example.name === "mcp" && example.features.includes("mcp")));
});

test("all example runs share the same required feature graph", () => {
  const featureArguments = [];
  runExamples({
    examples: [{name:"skills",features:["skills"]},{name:"mcp",features:["mcp"]}],
    environment: {},
    run(_command,args) {
      featureArguments.push(args[args.indexOf("--features") + 1]);
      const name = args[args.indexOf("--example") + 1];
      return {status:0,stdout:`EXAMPLE_OK ${name}\n`,stderr:""};
    },
  });
  assert.deepEqual(featureArguments, ["mcp,skills", "mcp,skills"]);
});


test("fleet measurements retain their declared release and minimal-feature invocation", () => {
  const examples = discoverExamples().filter(({name}) => name === "linux_fleet" || name === "mcp");
  const fleet = examples.find(({name}) => name === "linux_fleet");
  assert.equal(fleet.profile, "release");
  assert.equal(fleet.defaultFeatures, false);
  const calls = new Map();
  runExamples({examples, environment: {}, run(_command, args) {
    const name = args[args.indexOf("--example") + 1];
    calls.set(name, args);
    return {status: 0, stdout: `EXAMPLE_OK ${name}\n`, stderr: ""};
  }});
  assert.ok(calls.get("linux_fleet").includes("--release"));
  assert.ok(calls.get("linux_fleet").includes("--no-default-features"));
  assert.equal(calls.get("linux_fleet").includes("--features"), false);
  assert.equal(calls.get("mcp").includes("--release"), false);
  assert.equal(calls.get("mcp").includes("--no-default-features"), false);
  assert.ok(calls.get("mcp").includes("mcp"));
});
