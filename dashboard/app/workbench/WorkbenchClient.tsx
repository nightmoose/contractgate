"use client";

/**
 * WorkbenchClient — RFC-046: API Workbench.
 *
 * Browser-local execution only. No API calls pass through ContractGate
 * servers; credentials never leave the browser.
 *
 * Free tier: Try It mode — 1 endpoint, inference visible, Save/Deploy/Export
 * disabled with upsell tooltip.
 * Growth+: full suite, deploy, export, drift detection.
 */

import { useState, useCallback, useRef, useEffect } from "react";
import clsx from "clsx";
import jsYaml from "js-yaml";
import { useOrg, planAtLeast } from "@/lib/org";
import { deployContract } from "@/lib/api";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
type SeedMode = "url" | "spec-url" | "upload" | "curl" | "postman" | "bruno";
type FieldType = "string" | "integer" | "number" | "boolean" | "array" | "object" | "any";
type TemporalType = "date" | "datetime" | "timestamp";
type AuthType = "none" | "bearer" | "api-key" | "basic";
type OdcsVersion = "latest" | "2.2.2" | "2.1.0" | "2.0.0";

interface QueryParam {
  key: string;
  value: string;
  enabled: boolean;
}

interface PathParam {
  key: string;
  value: string;
}

interface EndpointDef {
  id: string;
  method: HttpMethod;
  path: string;
  summary?: string;
  pathParams: PathParam[];
  queryParams: QueryParam[];
  bodySchema?: unknown;
}

interface AuthConfig {
  type: AuthType;
  // Credentials stored in sessionStorage only, not in this object.
}

interface InferredField {
  name: string;
  type: FieldType;
  required: boolean;
  confidence: number; // 0-100
  pattern?: string;
  enum?: string[];
  min?: number;
  max?: number;
  temporalType?: TemporalType;
  pii: boolean;
  annotation?: string;
  // Refinement overrides
  overrideType?: FieldType;
  overrideRequired?: boolean;
  overridePattern?: string;
  overrideEnum?: string;
  overrideMin?: string;
  overrideMax?: string;
  overrideTemporalType?: TemporalType | "";
  overridePii?: boolean;
  overrideAnnotation?: string;
}

interface ApiResponse {
  status: number;
  statusText: string;
  body: unknown;
  rawBody: string;
  headers: Record<string, string>;
  durationMs: number;
}

interface SuiteEntry {
  endpoint: EndpointDef;
  fields: InferredField[];
  contractName: string;
}

interface DriftResult {
  added: string[];
  removed: string[];
  typeChanged: Array<{ name: string; was: FieldType; now: FieldType }>;
  enumChanged: Array<{ name: string; newValues: string[] }>;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const METHOD_COLORS: Record<HttpMethod, string> = {
  GET:     "bg-green-900/40 text-green-400 border-green-700/50",
  POST:    "bg-blue-900/40 text-blue-400 border-blue-700/50",
  PUT:     "bg-amber-900/40 text-amber-400 border-amber-700/50",
  PATCH:   "bg-purple-900/40 text-purple-400 border-purple-700/50",
  DELETE:  "bg-red-900/40 text-red-400 border-red-700/50",
  HEAD:    "bg-slate-700/40 text-slate-400 border-slate-600/50",
  OPTIONS: "bg-slate-700/40 text-slate-400 border-slate-600/50",
};

const ODCS_VERSIONS: OdcsVersion[] = ["latest", "2.2.2", "2.1.0", "2.0.0"];

const SESSION_KEY = "wb_session_v1";
const AUTH_KEY    = "wb_auth_v1"; // sessionStorage only

// ---------------------------------------------------------------------------
// Inference helpers
// ---------------------------------------------------------------------------

const UUID_RE    = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DATE_RE    = /^\d{4}-\d{2}-\d{2}$/;
const DT_RE      = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/;
const EMAIL_RE   = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const TS_NUM_RE  = /^\d{10,13}$/; // unix timestamp (10=seconds, 13=ms)

function detectType(value: unknown): { type: FieldType; confidence: number; pattern?: string; temporalType?: TemporalType; enumHint?: string } {
  if (value === null || value === undefined) return { type: "string", confidence: 10 };
  if (typeof value === "boolean") return { type: "boolean", confidence: 90 };
  if (typeof value === "number") return { type: Number.isInteger(value) ? "integer" : "number", confidence: 85 };
  if (Array.isArray(value)) return { type: "array", confidence: 75 };
  if (typeof value === "object") return { type: "object", confidence: 75 };
  if (typeof value === "string") {
    if (UUID_RE.test(value))  return { type: "string", confidence: 95, pattern: "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" };
    if (DT_RE.test(value))    return { type: "string", confidence: 90, temporalType: "datetime" };
    if (DATE_RE.test(value))  return { type: "string", confidence: 90, temporalType: "date" };
    if (TS_NUM_RE.test(value)) return { type: "integer", confidence: 70, temporalType: "timestamp" };
    if (EMAIL_RE.test(value)) return { type: "string", confidence: 88, pattern: "^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$" };
    return { type: "string", confidence: 70 };
  }
  return { type: "any", confidence: 30 };
}

function inferFields(body: unknown): InferredField[] {
  // Unwrap top-level array → use first element
  const target = Array.isArray(body) ? body[0] : body;
  if (!target || typeof target !== "object" || Array.isArray(target)) return [];

  const samples = Array.isArray(body) ? body.slice(0, 10) : [body];
  const obj = target as Record<string, unknown>;

  return Object.entries(obj).map(([name, value]) => {
    const { type, confidence: baseConf, pattern, temporalType } = detectType(value);

    // Bump confidence if field appears in all samples
    const presentInAll = samples.every(s => s && typeof s === "object" && !Array.isArray(s) && name in (s as object));
    const nonNullInAll = samples.every(s => {
      const v = (s as Record<string, unknown>)[name];
      return v !== null && v !== undefined;
    });

    // Collect string values across samples for enum hint
    const strVals = samples
      .map(s => (s as Record<string, unknown>)[name])
      .filter(v => typeof v === "string") as string[];
    const uniqueVals = [...new Set(strVals)];
    const enumVals = uniqueVals.length > 1 && uniqueVals.length <= 8 && strVals.length >= 2
      ? uniqueVals
      : undefined;

    const confidence = Math.min(100,
      baseConf +
      (presentInAll ? 5 : -15) +
      (nonNullInAll ? 0 : -20)
    );

    return {
      name,
      type,
      required: nonNullInAll && presentInAll,
      confidence,
      pattern,
      enum: enumVals,
      temporalType,
      pii: false,
    };
  });
}

// ---------------------------------------------------------------------------
// YAML generation
// ---------------------------------------------------------------------------

function fieldEffective(f: InferredField) {
  return {
    type:         f.overrideType        ?? f.type,
    required:     f.overrideRequired    ?? f.required,
    pattern:      f.overridePattern     ?? f.pattern,
    enum:         f.overrideEnum        ? f.overrideEnum.split(",").map(v => v.trim()).filter(Boolean) : f.enum,
    min:          f.overrideMin         ? parseFloat(f.overrideMin) : f.min,
    max:          f.overrideMax         ? parseFloat(f.overrideMax) : f.max,
    temporalType: (f.overrideTemporalType !== undefined ? f.overrideTemporalType : f.temporalType) || undefined,
    pii:          f.overridePii         ?? f.pii,
    annotation:   f.overrideAnnotation  ?? f.annotation,
  };
}

function generateYaml(name: string, description: string, fields: InferredField[]): string {
  const lines: string[] = [
    `version: "1.0"`,
    `name: "${name}"`,
    `description: "${description}"`,
    ``,
    `ontology:`,
    `  entities:`,
  ];
  for (const f of fields) {
    const e = fieldEffective(f);
    lines.push(`    - name: ${f.name}`);
    lines.push(`      type: ${e.type}`);
    lines.push(`      required: ${e.required}`);
    if (e.pattern)      lines.push(`      pattern: "${e.pattern}"`);
    if (e.enum?.length) lines.push(`      enum: [${e.enum.map(v => `"${v}"`).join(", ")}]`);
    if (e.min !== undefined) lines.push(`      min: ${e.min}`);
    if (e.max !== undefined) lines.push(`      max: ${e.max}`);
    if (e.temporalType) lines.push(`      temporal_type: ${e.temporalType}`);
  }

  const piiFields = fields.filter(f => fieldEffective(f).pii);
  if (piiFields.length > 0) {
    lines.push(``, `glossary:`);
    for (const f of piiFields) {
      const e = fieldEffective(f);
      lines.push(`  - field: ${f.name}`);
      lines.push(`    description: "${e.annotation || "PII field — handle with care"}"`);
      lines.push(`    constraints: "pii: true"`);
    }
  }
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// ODCS generation
// ---------------------------------------------------------------------------

function generateOdcsYaml(name: string, description: string, fields: InferredField[], version: OdcsVersion): string {
  const ver = version === "latest" ? "2.2.2" : version;
  const schema: Record<string, unknown> = {
    dataContractSpecification: ver,
    id: `urn:contractgate:${name.replace(/\s+/g, "-").toLowerCase()}`,
    info: { title: name, version: "1.0.0", description },
    models: {
      [name]: {
        description,
        fields: Object.fromEntries(
          fields.map(f => {
            const e = fieldEffective(f);
            const odcsField: Record<string, unknown> = {
              type: e.type === "integer" ? "integer" : e.type === "number" ? "number" : "string",
              required: e.required,
              description: e.annotation || `Field: ${f.name}`,
            };
            if (e.pattern)      odcsField.pattern = e.pattern;
            if (e.enum?.length) odcsField.enum = e.enum;
            if (e.pii)          odcsField.classification = "pii";
            return [f.name, odcsField];
          })
        ),
      },
    },
  };
  return jsYaml.dump(schema, { lineWidth: 120 });
}

// ---------------------------------------------------------------------------
// Postman / Bruno export
// ---------------------------------------------------------------------------

function generatePostmanCollection(baseUrl: string, entries: SuiteEntry[]): string {
  const postScript = [
    "// ContractGate Workbench — post-response inference hook",
    "// Pipe this output to: contractgate infer --from-newman response.json --out contracts/out.yaml",
    "const body = pm.response.json();",
    "pm.test('Response is object', () => pm.expect(body).to.be.an('object'));",
  ].join("\n");

  const collection = {
    info: { name: "ContractGate Workbench Export", schema: "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
    item: entries.map(({ endpoint, contractName }) => ({
      name: `${endpoint.method} ${endpoint.path} [${contractName}]`,
      request: {
        method: endpoint.method,
        url: {
          raw: `${baseUrl}${endpoint.path}`,
          host: [baseUrl],
          path: endpoint.path.split("/").filter(Boolean),
          query: endpoint.queryParams.filter(q => q.enabled).map(q => ({ key: q.key, value: q.value })),
        },
        auth: { type: "bearer", bearer: [{ key: "token", value: "{{BEARER_TOKEN}}", type: "string" }] },
      },
      event: [{ listen: "test", script: { exec: [postScript], type: "text/javascript" } }],
    })),
  };
  return JSON.stringify(collection, null, 2);
}

function generateBrunoCollection(entries: SuiteEntry[]): string {
  return entries.map(({ endpoint, contractName }) => [
    `meta {`,
    `  name: ${endpoint.method} ${endpoint.path} [${contractName}]`,
    `  type: http`,
    `  seq: 1`,
    `}`,
    ``,
    `${endpoint.method.toLowerCase()} {`,
    `  url: {{baseUrl}}${endpoint.path}`,
    `  body: none`,
    `  auth: bearer`,
    `}`,
    ``,
    `auth:bearer {`,
    `  token: {{BEARER_TOKEN}}`,
    `}`,
    ``,
    `script:post-response {`,
    `  // Pipe Newman JSON output to: contractgate infer --from-newman response.json`,
    `  console.log(JSON.stringify(res.body));`,
    `}`,
  ].join("\n")).join("\n\n---\n\n");
}

// ---------------------------------------------------------------------------
// OpenAPI parser (client-side)
// ---------------------------------------------------------------------------

interface OpenApiSpec {
  openapi?: string;
  swagger?: string;
  paths?: Record<string, Record<string, {
    summary?: string;
    operationId?: string;
    parameters?: Array<{ name: string; in: string; required?: boolean }>;
    requestBody?: unknown;
  }>>;
  basePath?: string;
  servers?: Array<{ url: string }>;
}

function parseOpenApiEndpoints(spec: OpenApiSpec): EndpointDef[] {
  const endpoints: EndpointDef[] = [];
  if (!spec.paths) return endpoints;
  for (const [path, methods] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(methods)) {
      const m = method.toUpperCase() as HttpMethod;
      if (!["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS"].includes(m)) continue;
      const params = op.parameters ?? [];
      endpoints.push({
        id: `${m}:${path}`,
        method: m,
        path,
        summary: op.summary ?? op.operationId,
        pathParams: params
          .filter(p => p.in === "path")
          .map(p => ({ key: p.name, value: "" })),
        queryParams: params
          .filter(p => p.in === "query")
          .map(p => ({ key: p.name, value: "", enabled: !!p.required })),
        bodySchema: op.requestBody,
      });
    }
  }
  return endpoints;
}

function parseCurl(raw: string): EndpointDef | null {
  const urlMatch = raw.match(/curl\s+(?:-X\s+\w+\s+)?['"]?(https?:\/\/[^\s'"]+)['"]?/i);
  const methodMatch = raw.match(/-X\s+(\w+)/i);
  if (!urlMatch) return null;
  const fullUrl = urlMatch[1];
  let url: URL;
  try { url = new URL(fullUrl); } catch { return null; }
  const method = (methodMatch?.[1]?.toUpperCase() ?? "GET") as HttpMethod;
  const queryParams: QueryParam[] = [];
  url.searchParams.forEach((value, key) => queryParams.push({ key, value, enabled: true }));
  return {
    id: `${method}:${url.pathname}`,
    method,
    path: url.pathname,
    pathParams: [],
    queryParams,
  };
}

function parsePostmanCollection(raw: string): { baseUrl: string; endpoints: EndpointDef[] } {
  let col: unknown;
  try { col = JSON.parse(raw); } catch { return { baseUrl: "", endpoints: [] }; }
  const c = col as { info?: unknown; item?: unknown[] };
  if (!c.item) return { baseUrl: "", endpoints: [] };
  const endpoints: EndpointDef[] = [];
  for (const item of c.item) {
    const i = item as { name?: string; request?: { method?: string; url?: { raw?: string; path?: string[]; query?: Array<{ key?: string; value?: string; disabled?: boolean }> } } };
    if (!i.request) continue;
    const urlRaw = i.request.url?.raw ?? "";
    let parsed: URL | null = null;
    try { parsed = new URL(urlRaw); } catch { /* ignore */ }
    const path = parsed?.pathname ?? "/" + (i.request.url?.path?.join("/") ?? "");
    const method = (i.request.method?.toUpperCase() ?? "GET") as HttpMethod;
    const queryParams: QueryParam[] = (i.request.url?.query ?? []).map(q => ({
      key: q.key ?? "", value: q.value ?? "", enabled: !q.disabled,
    }));
    endpoints.push({ id: `${method}:${path}`, method, path, pathParams: [], queryParams });
  }
  const baseUrl = endpoints.length > 0 && c.item?.[0]
    ? (() => {
        const i = c.item![0] as { request?: { url?: { raw?: string } } };
        try { const u = new URL(i.request?.url?.raw ?? ""); return `${u.protocol}//${u.host}`; } catch { return ""; }
      })()
    : "";
  return { baseUrl, endpoints };
}

// ---------------------------------------------------------------------------
// UI sub-components
// ---------------------------------------------------------------------------

function MethodBadge({ method }: { method: HttpMethod }) {
  return (
    <span className={clsx("inline-block px-1.5 py-0.5 rounded text-[10px] font-bold border font-mono w-14 text-center shrink-0", METHOD_COLORS[method])}>
      {method}
    </span>
  );
}

function ConfidenceBar({ value }: { value: number }) {
  const color = value >= 70 ? "bg-green-500" : value >= 40 ? "bg-amber-500" : "bg-red-500";
  return (
    <div className="flex items-center gap-1.5">
      <div className="w-20 h-1.5 bg-[#1f2937] rounded-full overflow-hidden">
        <div className={clsx("h-full rounded-full", color)} style={{ width: `${value}%` }} />
      </div>
      <span className={clsx("text-[10px] font-mono", value >= 70 ? "text-green-400" : value >= 40 ? "text-amber-400" : "text-red-400")}>
        {value}
      </span>
    </div>
  );
}

function DisabledTooltipBtn({ label, tooltip, className }: { label: string; tooltip: string; className?: string }) {
  const [show, setShow] = useState(false);
  return (
    <div className="relative inline-block">
      <button
        disabled
        className={clsx("px-4 py-2 rounded-lg text-sm font-medium opacity-40 cursor-not-allowed", className)}
        onMouseEnter={() => setShow(true)}
        onMouseLeave={() => setShow(false)}
      >
        {label}
      </button>
      {show && (
        <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-1.5 bg-[#111827] border border-[#374151] rounded-lg text-xs text-slate-300 whitespace-nowrap z-50 pointer-events-none">
          {tooltip}
          <div className="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-[#374151]" />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export default function WorkbenchClient() {
  const { org, loading: orgLoading } = useOrg();
  const isGrowth = !orgLoading && !!org && planAtLeast(org.plan, "growth");
  const isFree   = !orgLoading && !!org && !isGrowth;

  // ---- Session state (persisted to localStorage, no creds) ----
  const [baseUrl,    setBaseUrl]    = useState("");
  const [endpoints,  setEndpoints]  = useState<EndpointDef[]>([]);
  const [selected,   setSelected]   = useState<EndpointDef | null>(null);
  const [bodyInput,  setBodyInput]  = useState("{}");
  const [response,   setResponse]   = useState<ApiResponse | null>(null);
  const [fields,     setFields]     = useState<InferredField[]>([]);
  const [suite,      setSuite]      = useState<SuiteEntry[]>([]);
  const [suiteName,  setSuiteName]  = useState("My API Suite");
  const [seedMode,   setSeedMode]   = useState<SeedMode>("url");
  const [seedInput,  setSeedInput]  = useState("");
  const [seedError,  setSeedError]  = useState("");
  const [isSending,  setIsSending]  = useState(false);
  const [isSeeding,  setIsSeeding]  = useState(false);
  const [expandedField, setExpandedField] = useState<string | null>(null);
  const [showExport, setShowExport] = useState(false);
  const [odcsVersion, setOdcsVersion] = useState<OdcsVersion>("latest");
  const [deployState, setDeployState] = useState<"idle" | "loading" | "ok" | "err">("idle");
  const [deployMsg,   setDeployMsg]  = useState("");
  const [contractName, setContractName] = useState("");
  const [contractDesc, setContractDesc] = useState("");
  const [driftResult,  setDriftResult] = useState<DriftResult | null>(null);
  const [isDrifting,   setIsDrifting]  = useState(false);
  // Free teaser: track endpoint call count
  const callCount = useRef(0);

  // ---- Auth (creds in sessionStorage only) ----
  const [authType,   setAuthType]   = useState<AuthType>("none");
  const [authValue,  setAuthValue]  = useState(""); // bearer token / api key / base64 basic

  // Persist non-cred session state
  useEffect(() => {
    try {
      const saved = localStorage.getItem(SESSION_KEY);
      if (saved) {
        const s = JSON.parse(saved);
        if (s.baseUrl)    setBaseUrl(s.baseUrl);
        if (s.endpoints)  setEndpoints(s.endpoints);
        if (s.suite)      setSuite(s.suite);
        if (s.suiteName)  setSuiteName(s.suiteName);
        if (s.contractName) setContractName(s.contractName);
        if (s.contractDesc) setContractDesc(s.contractDesc);
      }
      const auth = sessionStorage.getItem(AUTH_KEY);
      if (auth) {
        const a = JSON.parse(auth);
        if (a.type)  setAuthType(a.type);
        if (a.value) setAuthValue(a.value);
      }
    } catch { /* ignore */ }
  }, []);

  const persist = useCallback((patch: Record<string, unknown>) => {
    try {
      const saved = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "{}");
      localStorage.setItem(SESSION_KEY, JSON.stringify({ ...saved, ...patch }));
    } catch { /* ignore */ }
  }, []);

  const persistAuth = useCallback((type: AuthType, value: string) => {
    try { sessionStorage.setItem(AUTH_KEY, JSON.stringify({ type, value })); } catch { /* ignore */ }
  }, []);

  // ---- Seeding ----
  const handleSeed = useCallback(async () => {
    setSeedError("");
    setIsSeeding(true);
    try {
      if (seedMode === "url" || seedMode === "spec-url") {
        // Try to fetch OpenAPI spec
        const url = seedMode === "url"
          ? (seedInput.endsWith("/") ? seedInput : seedInput + "/") + "openapi.json"
          : seedInput;
        let specText: string;
        try {
          const res = await fetch(url, { headers: { Accept: "application/json, application/yaml" } });
          specText = await res.text();
        } catch {
          setSeedError("Could not fetch spec — check URL and CORS policy.");
          return;
        }
        let spec: OpenApiSpec;
        try { spec = JSON.parse(specText) as OpenApiSpec; }
        catch { try { spec = jsYaml.load(specText) as OpenApiSpec; } catch { setSeedError("Could not parse spec as JSON or YAML."); return; } }
        const discovered = parseOpenApiEndpoints(spec);
        if (discovered.length === 0) { setSeedError("No endpoints found in spec."); return; }
        const base = spec.servers?.[0]?.url ?? (seedMode === "url" ? seedInput : "");
        setBaseUrl(base);
        setEndpoints(discovered);
        persist({ baseUrl: base, endpoints: discovered });
      } else if (seedMode === "curl") {
        const ep = parseCurl(seedInput);
        if (!ep) { setSeedError("Could not parse curl command — check format."); return; }
        const urlMatch = seedInput.match(/https?:\/\/[^/\s'"]+/i);
        const base = urlMatch ? urlMatch[0] : "";
        setBaseUrl(base);
        setEndpoints([ep]);
        persist({ baseUrl: base, endpoints: [ep] });
      } else if (seedMode === "postman") {
        const { baseUrl: base, endpoints: eps } = parsePostmanCollection(seedInput);
        if (eps.length === 0) { setSeedError("No requests found in Postman collection."); return; }
        setBaseUrl(base);
        setEndpoints(eps);
        persist({ baseUrl: base, endpoints: eps });
      } else if (seedMode === "bruno") {
        setSeedError("Bruno import: paste as Postman JSON export or use CLI mode.");
        return;
      } else if (seedMode === "upload") {
        setSeedError("Upload a spec file using the file picker above, then click Explore.");
        return;
      }
    } finally {
      setIsSeeding(false);
    }
  }, [seedMode, seedInput, persist]);

  const handleFileUpload = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      setSeedInput(ev.target?.result as string ?? "");
      setSeedMode("postman"); // will parse as JSON/YAML spec
    };
    reader.readAsText(file);
  }, []);

  // ---- Sending ----
  const resolveUrl = useCallback((ep: EndpointDef): string => {
    let path = ep.path;
    for (const p of ep.pathParams) {
      path = path.replace(`{${p.key}}`, encodeURIComponent(p.value || p.key));
    }
    const qs = ep.queryParams
      .filter(q => q.enabled && q.key)
      .map(q => `${encodeURIComponent(q.key)}=${encodeURIComponent(q.value)}`)
      .join("&");
    return `${baseUrl}${path}${qs ? "?" + qs : ""}`;
  }, [baseUrl]);

  const buildHeaders = useCallback((): Record<string, string> => {
    const h: Record<string, string> = { "Content-Type": "application/json" };
    if (authType === "bearer" && authValue) h["Authorization"] = `Bearer ${authValue}`;
    if (authType === "api-key" && authValue)  h["X-API-Key"] = authValue;
    if (authType === "basic"  && authValue)   h["Authorization"] = `Basic ${authValue}`;
    return h;
  }, [authType, authValue]);

  const handleSend = useCallback(async () => {
    if (!selected) return;
    if (isFree && callCount.current >= 1) return; // Try It limit
    setIsSending(true);
    setResponse(null);
    setFields([]);
    setDriftResult(null);
    const start = Date.now();
    try {
      const fetchOpts: RequestInit = {
        method: selected.method,
        headers: buildHeaders(),
      };
      if (["POST","PUT","PATCH"].includes(selected.method) && bodyInput.trim() !== "{}") {
        fetchOpts.body = bodyInput;
      }
      const res = await fetch(resolveUrl(selected), fetchOpts);
      const rawBody = await res.text();
      let body: unknown = rawBody;
      try { body = JSON.parse(rawBody); } catch { /* keep as string */ }
      const respHeaders: Record<string, string> = {};
      res.headers.forEach((v, k) => { respHeaders[k] = v; });
      const apiResp: ApiResponse = {
        status: res.status,
        statusText: res.statusText,
        body,
        rawBody,
        headers: respHeaders,
        durationMs: Date.now() - start,
      };
      setResponse(apiResp);
      callCount.current += 1;
      // Auto-infer
      if (typeof body === "object" && body !== null) {
        const inferred = inferFields(body);
        setFields(inferred);
        if (!contractName && selected) {
          const n = selected.path.replace(/^\//, "").replace(/\//g, "_").replace(/[{}]/g, "").replace(/\s+/g, "_") || "my_api";
          setContractName(n);
        }
      }
    } catch (err) {
      setResponse({
        status: 0,
        statusText: String(err),
        body: null,
        rawBody: "",
        headers: {},
        durationMs: Date.now() - start,
      });
    } finally {
      setIsSending(false);
    }
  }, [selected, isFree, buildHeaders, resolveUrl, bodyInput, contractName]);

  // ---- Drift detection ----
  const handleDrift = useCallback(async () => {
    if (!selected || suite.length === 0) return;
    const existing = suite.find(s => s.endpoint.id === selected.id);
    if (!existing) return;
    setIsDrifting(true);
    await handleSend();
    setIsDrifting(false);
  }, [selected, suite, handleSend]);

  useEffect(() => {
    if (!isDrifting || fields.length === 0 || suite.length === 0 || !selected) return;
    const existing = suite.find(s => s.endpoint.id === selected.id);
    if (!existing) return;
    const oldNames = new Set(existing.fields.map(f => f.name));
    const newNames = new Set(fields.map(f => f.name));
    const added   = [...newNames].filter(n => !oldNames.has(n));
    const removed  = [...oldNames].filter(n => !newNames.has(n));
    const typeChanged = fields
      .filter(f => oldNames.has(f.name))
      .flatMap(f => {
        const old = existing.fields.find(o => o.name === f.name);
        return old && old.type !== f.type ? [{ name: f.name, was: old.type, now: f.type }] : [];
      });
    const enumChanged = fields
      .filter(f => f.enum && oldNames.has(f.name))
      .flatMap(f => {
        const old = existing.fields.find(o => o.name === f.name);
        const oldEnum = old?.enum ?? [];
        const newVals = (f.enum ?? []).filter(v => !oldEnum.includes(v));
        return newVals.length > 0 ? [{ name: f.name, newValues: newVals }] : [];
      });
    setDriftResult({ added, removed, typeChanged, enumChanged });
  }, [isDrifting, fields, suite, selected]);

  // ---- Suite management ----
  const addToSuite = useCallback(() => {
    if (!selected || fields.length === 0) return;
    setSuite(prev => {
      const next = prev.filter(s => s.endpoint.id !== selected.id);
      return [...next, { endpoint: selected, fields: [...fields], contractName: contractName || selected.path }];
    });
    persist({ suite: suite });
  }, [selected, fields, contractName, suite, persist]);

  // ---- Deploy ----
  const handleDeploy = useCallback(async (yamlContent: string) => {
    setDeployState("loading");
    try {
      const res = await deployContract({ yaml_content: yamlContent, deployed_by: "workbench" });
      setDeployState("ok");
      setDeployMsg(`Deployed ${res.name} @ ${res.version}`);
    } catch (err) {
      setDeployState("err");
      setDeployMsg(String(err));
    }
  }, []);

  // ---- Download helpers ----
  const download = useCallback((filename: string, content: string, mime = "text/plain") => {
    const blob = new Blob([content], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url; a.download = filename; a.click();
    URL.revokeObjectURL(url);
  }, []);

  // ---- Render ----
  const hasSession = endpoints.length > 0;
  const effectiveFields = fields;
  const currentYaml = contractName && effectiveFields.length > 0
    ? generateYaml(contractName, contractDesc || `Contract for ${contractName}`, effectiveFields)
    : "";
  const currentOdcs = contractName && effectiveFields.length > 0
    ? generateOdcsYaml(contractName, contractDesc || `Contract for ${contractName}`, effectiveFields, odcsVersion)
    : "";

  return (
    <div className="min-h-screen bg-[#0d1117] text-slate-100">
      {/* Header */}
      <div className="border-b border-[#1f2937] px-6 py-4 flex items-center justify-between">
        <div>
          <h1 className="text-lg font-bold text-slate-100">API Workbench</h1>
          <p className="text-xs text-slate-500 mt-0.5">Explore endpoints · infer contracts · enforce at the gate</p>
        </div>
        <div className="flex items-center gap-3">
          {isFree && (
            <span className="text-xs bg-amber-900/30 border border-amber-700/40 text-amber-400 px-2.5 py-1 rounded-full font-medium">
              Try It — 1 endpoint · <a href="/pricing" className="underline hover:text-amber-300">Upgrade to save</a>
            </span>
          )}
          {hasSession && (
            <button
              onClick={() => { setEndpoints([]); setSelected(null); setResponse(null); setFields([]); setSeedInput(""); setSeedError(""); callCount.current = 0; localStorage.removeItem(SESSION_KEY); }}
              className="text-xs text-slate-500 hover:text-slate-300 transition-colors"
            >
              ↩ New session
            </button>
          )}
        </div>
      </div>

      {/* Seed panel */}
      {!hasSession && (
        <div className="max-w-2xl mx-auto px-6 py-14">
          <h2 className="text-xl font-semibold text-slate-200 mb-2">Start from your API</h2>
          <p className="text-sm text-slate-500 mb-8">Paste a URL, spec, curl command, or Postman collection to discover endpoints.</p>

          {/* Mode tabs */}
          <div className="flex gap-1 mb-4 bg-[#111827] border border-[#1f2937] rounded-lg p-1">
            {(["url","spec-url","curl","postman","upload"] as SeedMode[]).map(m => (
              <button
                key={m}
                onClick={() => { setSeedMode(m); setSeedInput(""); setSeedError(""); }}
                className={clsx(
                  "flex-1 py-1.5 px-2 rounded-md text-xs font-medium transition-colors",
                  seedMode === m ? "bg-green-800/50 text-green-300" : "text-slate-500 hover:text-slate-300"
                )}
              >
                {({"url": "Base URL", "spec-url": "OpenAPI URL", "curl": "curl", "postman": "Postman", "upload": "Upload", "bruno": "Bruno"} as Record<SeedMode, string>)[m]}
              </button>
            ))}
          </div>

          {seedMode === "upload" ? (
            <label className="block border-2 border-dashed border-[#374151] rounded-xl p-10 text-center cursor-pointer hover:border-green-700/50 transition-colors">
              <div className="text-4xl mb-3">📂</div>
              <p className="text-sm text-slate-400">Drop or click to upload a <span className="text-slate-300 font-medium">.json</span> or <span className="text-slate-300 font-medium">.yaml</span> spec file</p>
              <input type="file" accept=".json,.yaml,.yml" className="hidden" onChange={handleFileUpload} />
            </label>
          ) : (
            <textarea
              value={seedInput}
              onChange={e => setSeedInput(e.target.value)}
              placeholder={{
                url: "https://api.example.com/v2",
                "spec-url": "https://api.example.com/openapi.json",
                curl: 'curl -X GET "https://api.example.com/users" -H "Authorization: Bearer TOKEN"',
                postman: "Paste Postman Collection v2.1 JSON here…",
                upload: "",
                bruno: "",
              }[seedMode]}
              className="w-full h-32 bg-[#111827] border border-[#1f2937] rounded-xl px-4 py-3 text-sm font-mono text-slate-300 placeholder:text-slate-600 resize-none focus:outline-none focus:border-green-700/60"
            />
          )}

          {seedError && <p className="mt-2 text-sm text-red-400">{seedError}</p>}

          <button
            onClick={handleSeed}
            disabled={isSeeding || (seedMode !== "upload" && !seedInput.trim())}
            className="mt-4 w-full py-2.5 bg-green-600 hover:bg-green-500 disabled:opacity-40 disabled:cursor-not-allowed text-white font-semibold rounded-lg text-sm transition-colors"
          >
            {isSeeding ? "Exploring…" : "Explore →"}
          </button>

          <p className="mt-4 text-xs text-slate-600 text-center">
            All API calls run in your browser. Credentials never reach ContractGate servers.
          </p>
        </div>
      )}

      {/* Main two-panel layout */}
      {hasSession && (
        <div className="flex h-[calc(100vh-65px)]">

          {/* Left: endpoint list */}
          <div className="w-64 shrink-0 border-r border-[#1f2937] overflow-y-auto bg-[#111827]">
            <div className="p-3 border-b border-[#1f2937]">
              <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">{endpoints.length} endpoints</p>
            </div>
            <div className="p-2 space-y-0.5">
              {endpoints.map(ep => (
                <button
                  key={ep.id}
                  onClick={() => { setSelected(ep); setResponse(null); setFields([]); setBodyInput("{}"); setShowExport(false); setDriftResult(null); }}
                  className={clsx(
                    "w-full text-left px-2.5 py-2 rounded-lg flex items-start gap-2 transition-colors",
                    selected?.id === ep.id ? "bg-green-900/30 border border-green-800/40" : "hover:bg-[#1f2937]"
                  )}
                >
                  <MethodBadge method={ep.method} />
                  <div className="min-w-0">
                    <p className="text-xs font-mono text-slate-300 truncate">{ep.path}</p>
                    {ep.summary && <p className="text-[10px] text-slate-600 truncate mt-0.5">{ep.summary}</p>}
                    {suite.some(s => s.endpoint.id === ep.id) && (
                      <span className="text-[9px] text-green-500">✓ in suite</span>
                    )}
                  </div>
                </button>
              ))}
            </div>
          </div>

          {/* Right: request + response */}
          <div className="flex-1 overflow-y-auto">
            {!selected ? (
              <div className="flex items-center justify-center h-full text-slate-600 text-sm">Select an endpoint →</div>
            ) : (
              <div className="p-5 space-y-5 max-w-4xl">

                {/* URL bar */}
                <div className="flex items-center gap-2">
                  <MethodBadge method={selected.method} />
                  <div className="flex-1 font-mono text-sm bg-[#111827] border border-[#1f2937] rounded-lg px-3 py-2 text-slate-300 truncate">
                    {resolveUrl(selected)}
                  </div>
                  <button
                    onClick={handleSend}
                    disabled={isSending || (isFree && callCount.current >= 1)}
                    className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-semibold rounded-lg transition-colors"
                  >
                    {isSending ? "Sending…" : "Send"}
                  </button>
                </div>

                {isFree && callCount.current >= 1 && (
                  <div className="text-xs text-amber-400 bg-amber-900/20 border border-amber-700/30 px-3 py-2 rounded-lg">
                    Try It limit reached — <a href="/pricing" className="underline">upgrade to Growth</a> to explore more endpoints.
                  </div>
                )}

                {/* Params */}
                {(selected.pathParams.length > 0 || selected.queryParams.length > 0) && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3">
                    <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">Parameters</p>
                    {selected.pathParams.map((p, i) => (
                      <div key={p.key} className="flex items-center gap-2">
                        <span className="text-xs text-slate-500 w-20 shrink-0 font-mono">{p.key}</span>
                        <input
                          value={p.value}
                          onChange={e => {
                            const next = { ...selected, pathParams: selected.pathParams.map((x, j) => j === i ? { ...x, value: e.target.value } : x) };
                            setSelected(next);
                          }}
                          placeholder="path value"
                          className="flex-1 bg-[#0d1117] border border-[#374151] rounded px-2 py-1 text-xs font-mono text-slate-300 focus:outline-none focus:border-green-700/60"
                        />
                      </div>
                    ))}
                    {selected.queryParams.map((q, i) => (
                      <div key={q.key + i} className="flex items-center gap-2">
                        <input
                          type="checkbox"
                          checked={q.enabled}
                          onChange={e => {
                            const next = { ...selected, queryParams: selected.queryParams.map((x, j) => j === i ? { ...x, enabled: e.target.checked } : x) };
                            setSelected(next);
                          }}
                          className="accent-green-500"
                        />
                        <span className="text-xs text-slate-500 w-20 shrink-0 font-mono">{q.key}</span>
                        <input
                          value={q.value}
                          onChange={e => {
                            const next = { ...selected, queryParams: selected.queryParams.map((x, j) => j === i ? { ...x, value: e.target.value } : x) };
                            setSelected(next);
                          }}
                          placeholder="value"
                          className="flex-1 bg-[#0d1117] border border-[#374151] rounded px-2 py-1 text-xs font-mono text-slate-300 focus:outline-none focus:border-green-700/60"
                        />
                      </div>
                    ))}
                  </div>
                )}

                {/* Auth */}
                <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3">
                  <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">Auth <span className="normal-case text-slate-600 font-normal">(stored in tab only)</span></p>
                  <div className="flex items-center gap-2">
                    {(["none","bearer","api-key","basic"] as AuthType[]).map(t => (
                      <button key={t} onClick={() => { setAuthType(t); persistAuth(t, authValue); }}
                        className={clsx("px-2.5 py-1 rounded text-xs font-medium transition-colors", authType === t ? "bg-green-800/50 text-green-300" : "text-slate-500 hover:text-slate-300 bg-[#0d1117]")}
                      >{t === "none" ? "None" : t === "bearer" ? "Bearer" : t === "api-key" ? "API Key" : "Basic"}</button>
                    ))}
                  </div>
                  {authType !== "none" && (
                    <input
                      type="password"
                      value={authValue}
                      onChange={e => { setAuthValue(e.target.value); persistAuth(authType, e.target.value); }}
                      placeholder={authType === "bearer" ? "your-token" : authType === "api-key" ? "your-api-key" : "base64(user:pass)"}
                      className="w-full bg-[#0d1117] border border-[#374151] rounded px-3 py-1.5 text-xs font-mono text-slate-300 focus:outline-none focus:border-green-700/60"
                    />
                  )}
                </div>

                {/* Body */}
                {["POST","PUT","PATCH"].includes(selected.method) && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4">
                    <p className="text-xs text-slate-500 font-medium uppercase tracking-wider mb-2">Request body</p>
                    <textarea
                      value={bodyInput}
                      onChange={e => setBodyInput(e.target.value)}
                      className="w-full h-28 bg-[#0d1117] border border-[#374151] rounded px-3 py-2 text-xs font-mono text-slate-300 resize-none focus:outline-none focus:border-green-700/60"
                    />
                  </div>
                )}

                {/* Response */}
                {response && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <span className={clsx("text-sm font-bold font-mono", response.status >= 200 && response.status < 300 ? "text-green-400" : response.status >= 400 ? "text-red-400" : "text-amber-400")}>
                          {response.status || "ERR"}
                        </span>
                        <span className="text-xs text-slate-500">{response.statusText}</span>
                        <span className="text-xs text-slate-600">{response.durationMs}ms</span>
                      </div>
                    </div>
                    <pre className="text-xs font-mono text-slate-300 bg-[#0d1117] rounded-lg p-3 overflow-x-auto max-h-64 whitespace-pre-wrap break-all">
                      {typeof response.body === "object"
                        ? JSON.stringify(response.body, null, 2)
                        : response.rawBody}
                    </pre>
                  </div>
                )}

                {/* Inferred fields */}
                {effectiveFields.length > 0 && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">Inferred schema — {effectiveFields.length} fields</p>
                      <div className="flex items-center gap-2">
                        {/* Contract name / description */}
                        <input
                          value={contractName}
                          onChange={e => setContractName(e.target.value)}
                          placeholder="contract name"
                          className="bg-[#0d1117] border border-[#374151] rounded px-2 py-1 text-xs font-mono text-slate-300 focus:outline-none focus:border-green-700/60 w-36"
                        />
                        <input
                          value={contractDesc}
                          onChange={e => setContractDesc(e.target.value)}
                          placeholder="description (optional)"
                          className="bg-[#0d1117] border border-[#374151] rounded px-2 py-1 text-xs text-slate-400 focus:outline-none focus:border-green-700/60 w-48"
                        />
                      </div>
                    </div>

                    {/* Drift result */}
                    {driftResult && (
                      <div className="bg-[#0d1117] border border-[#374151] rounded-lg p-3 text-xs space-y-1">
                        <p className="font-semibold text-slate-400 mb-2">Drift report</p>
                        {driftResult.added.map(n => <p key={n} className="text-green-400">+ {n} (new field)</p>)}
                        {driftResult.removed.map(n => <p key={n} className="text-red-400">− {n} (removed)</p>)}
                        {driftResult.typeChanged.map(c => <p key={c.name} className="text-amber-400">~ {c.name}: {c.was} → {c.now}</p>)}
                        {driftResult.enumChanged.map(c => <p key={c.name} className="text-amber-400">~ {c.name}: new enum values {c.newValues.join(", ")}</p>)}
                        {driftResult.added.length + driftResult.removed.length + driftResult.typeChanged.length + driftResult.enumChanged.length === 0 && (
                          <p className="text-slate-500">No drift detected.</p>
                        )}
                      </div>
                    )}

                    {/* Fields table */}
                    <div className="space-y-1">
                      {effectiveFields.map(f => (
                        <div key={f.name}>
                          <div
                            className={clsx(
                              "flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer transition-colors",
                              f.confidence < 40 ? "bg-red-900/10 border border-red-900/30" : f.confidence < 70 ? "bg-amber-900/10 border border-amber-900/30" : "bg-[#0d1117] border border-[#1f2937]",
                              expandedField === f.name && "border-green-800/40"
                            )}
                            onClick={() => setExpandedField(prev => prev === f.name ? null : f.name)}
                          >
                            <span className="font-mono text-xs text-slate-300 w-40 truncate">{f.name}</span>
                            <span className="text-[10px] bg-slate-700/50 text-slate-400 px-1.5 py-0.5 rounded font-mono">
                              {f.overrideType ?? f.type}
                            </span>
                            <ConfidenceBar value={f.confidence} />
                            <span className={clsx("text-[10px] ml-auto", (f.overrideRequired ?? f.required) ? "text-green-500" : "text-slate-600")}>
                              {(f.overrideRequired ?? f.required) ? "required" : "optional"}
                            </span>
                            {(f.overridePii ?? f.pii) && <span className="text-[9px] text-red-400 bg-red-900/20 border border-red-900/30 px-1 rounded">PII</span>}
                            {f.temporalType && <span className="text-[9px] text-purple-400 bg-purple-900/20 border border-purple-900/30 px-1 rounded">{f.overrideTemporalType || f.temporalType}</span>}
                            <span className="text-[10px] text-slate-600 ml-1">{expandedField === f.name ? "▲" : "▼"}</span>
                          </div>

                          {/* Refinement panel */}
                          {expandedField === f.name && (
                            <div className="ml-3 mt-1 p-4 bg-[#0d1117] border border-[#1f2937] rounded-lg grid grid-cols-2 gap-3 text-xs">
                              <label className="flex flex-col gap-1">
                                <span className="text-slate-500">Type</span>
                                <select
                                  value={f.overrideType ?? f.type}
                                  onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideType: e.target.value as FieldType } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 text-slate-300 focus:outline-none"
                                >
                                  {(["string","integer","number","boolean","array","object","any"] as FieldType[]).map(t => <option key={t} value={t}>{t}</option>)}
                                </select>
                              </label>
                              <label className="flex flex-col gap-1">
                                <span className="text-slate-500">Temporal type</span>
                                <select
                                  value={f.overrideTemporalType ?? f.temporalType ?? ""}
                                  onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideTemporalType: e.target.value as TemporalType | "" } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 text-slate-300 focus:outline-none"
                                >
                                  <option value="">— none —</option>
                                  <option value="date">date</option>
                                  <option value="datetime">datetime</option>
                                  <option value="timestamp">timestamp</option>
                                </select>
                              </label>
                              <label className="flex flex-col gap-1 col-span-2">
                                <span className="text-slate-500">Pattern (regex)</span>
                                <input value={f.overridePattern ?? f.pattern ?? ""} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overridePattern: e.target.value } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 font-mono text-slate-300 focus:outline-none" placeholder="^[a-z]+$" />
                              </label>
                              <label className="flex flex-col gap-1 col-span-2">
                                <span className="text-slate-500">Enum values (comma-separated)</span>
                                <input value={f.overrideEnum ?? (f.enum?.join(", ") ?? "")} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideEnum: e.target.value } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 font-mono text-slate-300 focus:outline-none" placeholder="active, inactive, pending" />
                              </label>
                              <label className="flex flex-col gap-1">
                                <span className="text-slate-500">Min</span>
                                <input type="number" value={f.overrideMin ?? f.min ?? ""} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideMin: e.target.value } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 text-slate-300 focus:outline-none" />
                              </label>
                              <label className="flex flex-col gap-1">
                                <span className="text-slate-500">Max</span>
                                <input type="number" value={f.overrideMax ?? f.max ?? ""} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideMax: e.target.value } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 text-slate-300 focus:outline-none" />
                              </label>
                              <div className="flex items-center gap-4 col-span-2">
                                <label className="flex items-center gap-2 cursor-pointer">
                                  <input type="checkbox" checked={f.overrideRequired ?? f.required} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideRequired: e.target.checked } : x))} className="accent-green-500" />
                                  <span className="text-slate-400">Required</span>
                                </label>
                                <label className="flex items-center gap-2 cursor-pointer">
                                  <input type="checkbox" checked={f.overridePii ?? f.pii} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overridePii: e.target.checked } : x))} className="accent-red-500" />
                                  <span className="text-slate-400">PII</span>
                                </label>
                              </div>
                              <label className="flex flex-col gap-1 col-span-2">
                                <span className="text-slate-500">Annotation / business rule</span>
                                <input value={f.overrideAnnotation ?? f.annotation ?? ""} onChange={e => setFields(prev => prev.map(x => x.name === f.name ? { ...x, overrideAnnotation: e.target.value } : x))}
                                  className="bg-[#111827] border border-[#374151] rounded px-2 py-1 text-slate-300 focus:outline-none" placeholder="e.g. must be non-negative USD amount" />
                              </label>
                            </div>
                          )}
                        </div>
                      ))}
                    </div>

                    {/* Action bar */}
                    <div className="flex items-center gap-2 pt-2 border-t border-[#1f2937]">
                      {isGrowth ? (
                        <>
                          <button onClick={addToSuite} className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-xs font-medium rounded-lg transition-colors border border-[#374151]">
                            + Add to suite
                          </button>
                          <button onClick={() => setShowExport(true)} className="px-3 py-1.5 bg-green-700 hover:bg-green-600 text-white text-xs font-medium rounded-lg transition-colors">
                            Export / Deploy →
                          </button>
                          {suite.some(s => s.endpoint.id === selected?.id) && (
                            <button onClick={handleDrift} disabled={isDrifting} className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-amber-400 text-xs font-medium rounded-lg transition-colors border border-amber-900/30 disabled:opacity-40">
                              {isDrifting ? "Checking…" : "Check drift"}
                            </button>
                          )}
                        </>
                      ) : (
                        <>
                          <DisabledTooltipBtn label="+ Add to suite" tooltip="Save and suite — Growth plan required · Upgrade to Growth" className="bg-[#1f2937] text-slate-400 border border-[#374151]" />
                          <DisabledTooltipBtn label="Export / Deploy →" tooltip="Export and deploy — Growth plan required · Upgrade to Growth" className="bg-green-900/40 text-green-600" />
                        </>
                      )}
                    </div>
                  </div>
                )}

                {/* Export panel */}
                {showExport && isGrowth && currentYaml && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-5 space-y-4">
                    <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">Export & Deploy</p>

                    {/* ContractGate YAML */}
                    <div>
                      <div className="flex items-center justify-between mb-1.5">
                        <span className="text-xs text-slate-400 font-medium">ContractGate YAML</span>
                        <button onClick={() => download(`${contractName}.yaml`, currentYaml)} className="text-[10px] text-green-400 hover:text-green-300">↓ Download</button>
                      </div>
                      <pre className="text-xs font-mono text-slate-300 bg-[#0d1117] rounded-lg p-3 overflow-x-auto max-h-48 whitespace-pre">{currentYaml}</pre>
                    </div>

                    {/* ODCS YAML */}
                    <div>
                      <div className="flex items-center justify-between mb-1.5">
                        <span className="text-xs text-slate-400 font-medium">ODCS-compatible YAML</span>
                        <div className="flex items-center gap-2">
                          <select
                            value={odcsVersion}
                            onChange={e => setOdcsVersion(e.target.value as OdcsVersion)}
                            className="bg-[#0d1117] border border-[#374151] rounded px-2 py-0.5 text-[10px] text-slate-400 focus:outline-none"
                          >
                            {ODCS_VERSIONS.map(v => <option key={v} value={v}>{v === "latest" ? "Latest (2.2.2)" : v}</option>)}
                          </select>
                          <button onClick={() => download(`${contractName}.odcs.yaml`, currentOdcs)} className="text-[10px] text-green-400 hover:text-green-300">↓ Download</button>
                        </div>
                      </div>
                      <pre className="text-xs font-mono text-slate-300 bg-[#0d1117] rounded-lg p-3 overflow-x-auto max-h-48 whitespace-pre">{currentOdcs}</pre>
                    </div>

                    {/* Deploy */}
                    <div className="flex items-center gap-3 pt-2 border-t border-[#1f2937]">
                      <button
                        onClick={() => handleDeploy(currentYaml)}
                        disabled={deployState === "loading"}
                        className="px-4 py-2 bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white text-sm font-semibold rounded-lg transition-colors"
                      >
                        {deployState === "loading" ? "Deploying…" : "Deploy to ContractGate"}
                      </button>
                      {deployState === "ok" && <span className="text-xs text-green-400">✓ {deployMsg}</span>}
                      {deployState === "err" && <span className="text-xs text-red-400">✗ {deployMsg}</span>}
                    </div>

                    {/* Postman / Bruno export */}
                    <div className="flex items-center gap-2 pt-2 border-t border-[#1f2937]">
                      <span className="text-xs text-slate-500">Offline / VPN:</span>
                      <button
                        onClick={() => {
                          const entries = suite.length > 0 ? suite : [{ endpoint: selected!, fields: effectiveFields, contractName }];
                          download("contractgate-workbench.postman_collection.json", generatePostmanCollection(baseUrl, entries), "application/json");
                        }}
                        className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-xs font-medium rounded-lg border border-[#374151] transition-colors"
                      >↓ Postman Collection</button>
                      <button
                        onClick={() => {
                          const entries = suite.length > 0 ? suite : [{ endpoint: selected!, fields: effectiveFields, contractName }];
                          download("contractgate-workbench.bru", generateBrunoCollection(entries), "text/plain");
                        }}
                        className="px-3 py-1.5 bg-[#1f2937] hover:bg-[#374151] text-slate-300 text-xs font-medium rounded-lg border border-[#374151] transition-colors"
                      >↓ Bruno Collection</button>
                    </div>

                    <p className="text-[10px] text-slate-600">
                      Run Newman collections locally:{" "}
                      <code className="font-mono">newman run collection.json --reporter-json-export response.json | contractgate infer --from-newman response.json --out contracts/{contractName}.yaml</code>
                    </p>
                  </div>
                )}

                {/* Suite panel */}
                {isGrowth && suite.length > 0 && (
                  <div className="bg-[#111827] border border-[#1f2937] rounded-xl p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <p className="text-xs text-slate-500 font-medium uppercase tracking-wider">Suite — {suite.length} contract{suite.length !== 1 ? "s" : ""}</p>
                      <input
                        value={suiteName}
                        onChange={e => setSuiteName(e.target.value)}
                        className="bg-[#0d1117] border border-[#374151] rounded px-2 py-1 text-xs text-slate-300 focus:outline-none w-48"
                      />
                    </div>
                    {suite.map(s => (
                      <div key={s.endpoint.id} className="flex items-center gap-2">
                        <MethodBadge method={s.endpoint.method} />
                        <span className="text-xs font-mono text-slate-400 flex-1 truncate">{s.endpoint.path}</span>
                        <span className="text-[10px] text-slate-600">{s.fields.length} fields</span>
                        <button onClick={() => setSuite(prev => prev.filter(x => x.endpoint.id !== s.endpoint.id))}
                          className="text-[10px] text-red-500 hover:text-red-400 transition-colors">remove</button>
                      </div>
                    ))}
                    <button
                      onClick={() => setShowExport(true)}
                      className="w-full py-2 bg-green-700 hover:bg-green-600 text-white text-xs font-semibold rounded-lg transition-colors"
                    >
                      Export suite ({suite.length} contracts) →
                    </button>
                  </div>
                )}

              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
