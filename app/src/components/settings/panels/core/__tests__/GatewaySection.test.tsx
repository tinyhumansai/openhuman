/**
 * Tests for the gateway form's draft-to-record conversion.
 *
 * This is where the two-axis model becomes a stored record, so it is where a
 * wrong answer produces a gateway that saves cleanly and then cannot activate.
 * The rendering around it is exercised by the panel's own tests.
 */
import { describe, expect, it } from 'vitest';

import { draftToGateway } from '../GatewaySection';

function draft(overrides: Partial<Parameters<typeof draftToGateway>[0]> = {}) {
  return {
    id: 'builder',
    label: 'Build server',
    where: 'here' as const,
    contained: true,
    image: 'openhuman-core:latest',
    binary: '/usr/local/bin/openhuman-core',
    destination: '',
    sshPort: '',
    identity: '',
    acceptNewHostKey: false,
    ...overrides,
  };
}

describe('draftToGateway', () => {
  it('builds the two axes independently', () => {
    const built = draftToGateway(draft({ where: 'ssh', destination: 'builder@example.com' }));

    expect('gateway' in built).toBe(true);
    if (!('gateway' in built)) return;
    expect(built.gateway.spec).toMatchObject({
      kind: 'box',
      reach: { kind: 'ssh', destination: 'builder@example.com' },
      confinement: { kind: 'docker', image: 'openhuman-core:latest' },
    });
  });

  it('omits optional SSH settings rather than sending empty ones', () => {
    // The shell falls back to the user's own SSH config for anything unset,
    // and an empty string is not "unset" -- it is a value `ssh` would be
    // handed.
    const built = draftToGateway(draft({ where: 'ssh', destination: 'host' }));

    if (!('gateway' in built)) throw new Error('expected a gateway');
    const spec = built.gateway.spec;
    if (spec.kind !== 'box' || spec.reach.kind !== 'ssh') throw new Error('expected an ssh box');
    expect(spec.reach.port).toBeUndefined();
    expect(spec.reach.identity).toBeUndefined();
    expect(spec.reach.acceptNewHostKey).toBeUndefined();
  });

  it('carries the SSH port when one is given', () => {
    const built = draftToGateway(draft({ where: 'ssh', destination: 'host', sshPort: '2222' }));

    if (!('gateway' in built)) throw new Error('expected a gateway');
    const spec = built.gateway.spec;
    if (spec.kind !== 'box' || spec.reach.kind !== 'ssh') throw new Error('expected an ssh box');
    expect(spec.reach.port).toBe(2222);
  });

  it('refuses SSH ports outside the valid TCP range', () => {
    // `0` and anything past 65535 pass a bare digit check but reach the shell
    // as invalid connection settings, failing later with a generic error.
    expect(draftToGateway(draft({ where: 'ssh', destination: 'h', sshPort: '0' }))).toEqual({
      error: 'portInvalid',
    });
    expect(draftToGateway(draft({ where: 'ssh', destination: 'h', sshPort: '65536' }))).toEqual({
      error: 'portInvalid',
    });
    expect(draftToGateway(draft({ where: 'ssh', destination: 'h', sshPort: '70000' }))).toEqual({
      error: 'portInvalid',
    });
    // The boundary values stay valid.
    const built = draftToGateway(draft({ where: 'ssh', destination: 'h', sshPort: '65535' }));
    if (!('gateway' in built)) throw new Error('expected a gateway');
    expect(built.gateway.spec).toMatchObject({ reach: { kind: 'ssh', port: 65535 } });
  });

  it('falls back to the id when no label was typed', () => {
    const built = draftToGateway(draft({ label: '' }));

    if (!('gateway' in built)) throw new Error('expected a gateway');
    expect(built.gateway.label).toBe('builder');
  });

  it('refuses the id reserved for the core inside this app', () => {
    // Taking it over would shadow the one gateway guaranteed to work, which is
    // the only way to make the app unreachable from its own UI.
    expect(draftToGateway(draft({ id: 'desktop' }))).toEqual({ error: 'idReserved' });
  });

  it('names what is missing rather than failing generically', () => {
    expect(draftToGateway(draft({ id: '' }))).toEqual({ error: 'idRequired' });
    expect(draftToGateway(draft({ where: 'ssh', destination: '' }))).toEqual({
      error: 'destinationRequired',
    });
    expect(draftToGateway(draft({ image: '' }))).toEqual({ error: 'imageRequired' });
    expect(draftToGateway(draft({ contained: false, binary: '' }))).toEqual({
      error: 'binaryRequired',
    });
    expect(draftToGateway(draft({ where: 'ssh', destination: 'h', sshPort: 'abc' }))).toEqual({
      error: 'portInvalid',
    });
  });

  it('builds an uncontained box from the binary path', () => {
    const built = draftToGateway(draft({ contained: false }));

    if (!('gateway' in built)) throw new Error('expected a gateway');
    expect(built.gateway.spec).toMatchObject({
      confinement: { kind: 'passthrough', binary: '/usr/local/bin/openhuman-core' },
    });
  });
});
