import { z } from "zod";

// CapIA: no server secrets. The editor needs no .env to run (local-first / offline-first).
const webEnvSchema = z.object({
	NODE_ENV: z.enum(["development", "production", "test"]).default("development"),
	NEXT_PUBLIC_SITE_URL: z.url().default("http://localhost:3000"),
});

export type WebEnv = z.infer<typeof webEnvSchema>;

export const webEnv = webEnvSchema.parse(process.env);
