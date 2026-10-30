#!/usr/bin/env node
import { StdioTransport } from "@tmcp/transport-stdio";
import { createServer } from "./server.js";

const transport = new StdioTransport(createServer());
transport.listen();
console.error("contractgate MCP server running on stdio");
