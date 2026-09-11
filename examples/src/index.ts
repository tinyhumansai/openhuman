import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import nacl from "tweetnacl";

export interface OpenHumanRuntime {
    getPrivateKey(): string;
    getRpcUrl(): string;
}

/**
 * OpenHuman Agent Identity Attestation
 * 
 * NO MOCKS. This tool forces the agent to cryptographically sign a challenge
 * payload using its Solana private key. This proves ownership of the wallet
 * bound to the Agent OS execution layer.
 */
export const openHumanAttestationTool = {
    name: "OPENHUMAN_ATTEST",
    description: "Cryptographically attests the Agent's identity via a Solana Ed25519 signature.",
    
    execute: async (runtime: OpenHumanRuntime, challengePayload: string): Promise<any> => {
        try {
            const privateKeyStr = runtime.getPrivateKey();
            if (!privateKeyStr) throw new Error("Missing Solana Private Key in Enclave.");

            // 1. Reconstruct Agent Wallet
            const keypair = Keypair.fromSecretKey(Buffer.from(JSON.parse(privateKeyStr)));
            const pubkey = keypair.publicKey.toBase58();

            // 2. Format the challenge exactly as standard Solana message signing dictates
            const message = new TextEncoder().encode(`OpenHuman Attestation Challenge:\n${challengePayload}`);
            
            // 3. Natively sign the message (No mocks, real Ed25519 cryptography)
            const signature = nacl.sign.detached(message, keypair.secretKey);
            const signatureHex = Buffer.from(signature).toString("hex");

            // 4. Verify locally to ensure strict compliance before returning
            const isValid = nacl.sign.detached.verify(
                message,
                signature,
                keypair.publicKey.toBytes()
            );

            if (!isValid) throw new Error("Cryptographic verification failed internally.");

            return {
                status: "ATTESTATION_SUCCESS",
                pubkey: pubkey,
                challenge: challengePayload,
                signature: signatureHex,
                network: "Solana",
                layer: "Agent OS"
            };

        } catch (error) {
            console.error("OpenHuman Attestation Failed:", error);
            return {
                status: "ATTESTATION_FAILED",
                error: error instanceof Error ? error.message : "Unknown Error"
            };
        }
    }
};

export default openHumanAttestationTool;
