// Lets `node --test` load the dashboard's extensionless TypeScript imports
// (`./auth-email-link`) and the `@/` alias. Types are stripped by Node itself.
import { register } from "node:module";

register("./ts-resolve-hooks.mjs", import.meta.url);
