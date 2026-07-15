# ContractGate Marketing Collateral List — H2 2026 (Customers Only)

**Focus:** Get paying customers. No acquisition / data-room / buyer decks in this list.  
**Audience for this doc:** Alex + design agents (Claude Design, Grok Imagine, human designers).  
**Product positioning (lock):**  
> Stop bad data **before** it hits the warehouse — semantic contracts at ingest, with quarantine and sub‑ms validation.

**Primary ICP:** Platform / data engineers at mid-market & growth companies with Kafka or high-volume HTTP APIs who feel pain from bad events.  
**Secondary:** Proptech / vertical API feed teams (MRI-style) if already warm.

**Brand cues (existing):** Live demo at app.datacontractgate.com; screenshots in `screenshots/`; logos `logo.png` / `logo_small.png`; dark product UI (dashboard). Prefer dark, technical, confident — not generic SaaS purple gradient unless asked.

---

## How to use this list

| Priority | Meaning |
|---|---|
| **P0** | Needed before serious outbound / pilot closes — make these first |
| **P1** | Needed to scale conversion after first pilots |
| **P2** | Nice once P0/P1 exist; don’t block sales |

**For designers:** each item has format, size, purpose, must-include content, and reference assets. Generate **visuals only** where noted; copy can be refined by Alex/Grok after.

---

# P0 — Core sales kit (build first)

## 1. Hero product image / OG share card
| | |
|---|---|
| **Purpose** | LinkedIn, Twitter/X, Slack, Open Graph when sharing links |
| **Format** | PNG + SVG if possible |
| **Size** | 1200×630 (OG) + 1080×1080 (square social) |
| **Must include** | ContractGate name/logo; tagline; visual of “bad event blocked at the gate” (not a wall of UI chrome) |
| **Avoid** | Patent language; competitor names; cluttered feature grid |
| **Refs** | `logo.png`, `screenshots/stream-demo-stats.png` for product feel |

---

## 2. 15-minute demo storyboard (slide strip or storyboard frames)
| | |
|---|---|
| **Purpose** | Script + visual sequence for Loom / live demo (designers produce frames; Alex records) |
| **Format** | 6–8 frames as 16:9 PNGs (1920×1080) or single Figma-style PDF strip |
| **Story beats (fixed)** | 1) Bad event arrives → 2) Contract rejects → 3) Clear violation → 4) Quarantine → 5) Fix / promote version → 6) Replay → 7) Green path → 8) “Before warehouse” punchline |
| **Must include** | Caption under each frame; product UI real or stylized consistent with dark dashboard |
| **Refs** | `screenshots/stream-demo-records.png`, `stream-demo-stats.png`, quarantine UX if available |

---

## 3. One-pager PDF — “ContractGate for data teams”
| | |
|---|---|
| **Purpose** | Attach to outbound email; leave-behind after calls |
| **Format** | PDF, **1 page**, letter or A4; also export PNG preview |
| **Size** | 8.5×11" or A4 |
| **Sections** | Problem (bad data after the lake) · Solution (at ingest) · How it works (3 steps) · Key capabilities (5 bullets max) · Proof (speed / demo URL) · CTA (book pilot / signup / email) |
| **Visual** | Small architecture diagram: Producers → **ContractGate** → Kafka/Warehouse; logo; dark or clean tech layout |
| **CTA** | app.datacontractgate.com · datacontractgate@nightmoose.com · “2-week pilot” |

---

## 4. Landing / hero section creative (web)
| | |
|---|---|
| **Purpose** | Top of marketing site or app marketing page (design mock; eng can implement later) |
| **Format** | Desktop mock 1440×900 + mobile 390×844 (PNG or PDF) |
| **Must include** | Headline, subhead, primary CTA (“Start free” / “Watch demo”), secondary CTA (“Book a pilot”), product screenshot or illustrated flow |
| **Headline options (pick one in design)** | “Stop bad data before it hits your warehouse” / “Semantic contracts at the gate” |
| **Refs** | `screenshots/visual-builder.png`, `inference-generator.png` |

---

## 5. Competitive comparison visual (one slide / one image)
| | |
|---|---|
| **Purpose** | Sales calls when “we already have GE / Soda / Monte Carlo / Schema Registry” comes up |
| **Format** | 16:9 PNG + PDF slide |
| **Size** | 1920×1080 |
| **Content** | Table or matrix: **When validation runs** · Real-time · Quarantine+replay · Semantic depth · Self-host free |
| **Columns** | ContractGate · Great Expectations · Soda · Monte Carlo · Schema Registry (or “Schema only”) |
| **Tone** | Fair, sharp, not trash-talky; ContractGate wins on **at ingest + quarantine** |

---

## 6. Pilot offer one-pager
| | |
|---|---|
| **Purpose** | Convert interested leads into a time-boxed pilot |
| **Format** | PDF, 1 page |
| **Must include** | What they get (2 weeks) · What we need (one feed/topic + 1 owner) · Success metric (“X bad events blocked” / “first contract in production”) · What “done” looks like · Price (e.g. free design partner / $N pilot credit toward Growth) · CTA + calendar/email |
| **Visual** | Simple timeline week 1 / week 2; logo; minimal chrome |

---

## 7. Email header / signature bar
| | |
|---|---|
| **Purpose** | Outbound & follow-ups look intentional |
| **Format** | PNG |
| **Size** | 600×200 (header) + 400×80 (signature) |
| **Must include** | Logo, short tagline, URL |

---

# P1 — Conversion & nurture (after P0)

## 8. Case study template (layout)
| | |
|---|---|
| **Purpose** | Fill with first customer numbers; design the **empty** professional template now |
| **Format** | PDF 2 pages + cover PNG |
| **Sections** | Customer (logo placeholder) · Challenge · Solution · Results (3 big metrics) · Quote · Stack · CTA |
| **Visual** | Metric callout boxes; optional small architecture |

---

## 9. “How it works” diagram set (3 diagrams)
| | |
|---|---|
| **Purpose** | Reuse in deck, site, one-pager, LinkedIn |
| **Format** | SVG + PNG, transparent background preferred |
| **Diagram A** | HTTP ingest path |
| **Diagram B** | Kafka path (producer → CG → topic / DLQ-quarantine story) |
| **Diagram C** | Contract lifecycle (draft → stable → deprecate) — keep simple |
| **Style** | Clean boxes/arrows; dark-mode friendly variants if easy |

---

## 10. Social post templates (carousel + single)
| | |
|---|---|
| **Purpose** | Weekly LinkedIn without redesigning every time |
| **Format** | 5 single-post templates 1080×1080 + 1 carousel (5 slides) 1080×1080 |
| **Themes** | Problem (bad data cost) · At-ingest vs post-hoc · Quarantine story · Demo CTA · Tip of the week |
| **Must leave** | Space for short text overlay; brand mark |

---

## 11. Feature vignettes (4 tiles)
| | |
|---|---|
| **Purpose** | Website features grid / LinkedIn / one-pager thumbnails |
| **Format** | 4× PNG 800×600 |
| **Tiles** | 1) Semantic YAML contracts · 2) Quarantine + replay · 3) Inference (JSON→YAML) · 4) Speed (Rust / sub-ms) |
| **Refs** | `screenshots/inference-generator.png`, `visual-builder.png` |

---

## 12. Pricing visual
| | |
|---|---|
| **Purpose** | Clear Free / Growth / Enterprise without dumping them on a messy screenshot |
| **Format** | 16:9 PNG + PDF |
| **Tiers** | Self-Hosted Free · Cloud Free · Growth ($299) · Enterprise (Custom) |
| **Highlight** | Growth as “most teams” |
| **Note for designer** | Match product truth: don’t invent features; if unsure, label “see site for limits” |

---

## 13. Walkthrough thumbnail set
| | |
|---|---|
| **Purpose** | YouTube/Loom covers for HTTP / Kafka / CSV / demo |
| **Format** | 4× 1280×720 PNG |
| **Titles** | “5‑min stream demo” · “HTTP ingest” · “Kafka path” · “From sample JSON to contract” |

---

# P2 — Later / scale (don’t start until P0 done)

## 14. Full sales deck (8–10 slides)
| | |
|---|---|
| **Purpose** | Longer discovery calls (not first-touch) |
| **Format** | PDF + PPTX or keynote-ready PDF |
| **Slide list** | Title · Problem · Cost of bad data · Solution · How it works · Demo screens · Differentiation · Pricing · Pilot · CTA |
| **Depends on** | #1–6, #9 |

---

## 15. Conference / meetup poster or table tent
| | |
|---|---|
| **Format** | 11×17" poster PDF + 6×4" table card |
| **Purpose** | In-person only if attending events |

---

## 16. Sticker / merch mark (optional)
| | |
|---|---|
| **Format** | Simple icon mark SVG (gate / check / contract motif) |
| **Purpose** | Brand recognition; secondary |

---

## 17. Vertical one-pager variant (e.g. proptech / API feeds)
| | |
|---|---|
| **Purpose** | Only if chasing Findigs/MRI-style deals |
| **Format** | Same as #3 with vertical problem language |
| **Note** | Reuse existing Findigs PDF only if still accurate; redesign if dated |

---

# Explicitly OUT of scope (customer campaign)

Do **not** spend design time on these for customer acquisition:

- Acquisition / M&A teaser deck  
- Data room branding  
- Full brand book / rebrand  
- Multi-language  
- App Store / mobile  
- SOC2 marketing (until certified)  
- Patent-forward posters  

---

# Suggested production order for design agents

```
Batch A (week 1)     →  #1 Hero/OG  ·  #3 One-pager  ·  #7 Email bar
Batch B (week 1–2)   →  #2 Demo storyboard  ·  #4 Landing hero  ·  #5 Comparison
Batch C (week 2)     →  #6 Pilot offer  ·  #9 Diagrams  ·  #11 Feature tiles
Batch D (week 3+)    →  #10 Social templates  ·  #12 Pricing  ·  #13 Thumbnails
Batch E (after first customer) →  #8 Case study template filled  ·  #14 Sales deck
```

---

# Copy bank (for designers to place — can refine later)

**Tagline:** Semantic contract enforcement at ingestion  

**Headline:** Stop bad data before it hits your warehouse  

**Subhead:** ContractGate validates every event against rich semantic contracts in real time — quarantine failures, replay when fixed, all before the lake.  

**Bullets (max 5 on one-pager):**  
- At-ingest validation (HTTP, Kafka, batch)  
- Semantic YAML contracts (types, patterns, enums, quality rules)  
- Quarantine + replay for bad events  
- Sub-millisecond validation (Rust)  
- Self-host free or managed cloud  

**CTA primary:** Start free → app.datacontractgate.com  
**CTA secondary:** Book a 2-week pilot → datacontractgate@nightmoose.com  

**URLs to show:**  
- https://app.datacontractgate.com  
- https://app.datacontractgate.com/stream-demo  
- https://github.com/nightmoose/contractgate (`make demo`)  

---

# Asset inventory checklist (tick when done)

**P0**
- [ ] 1 Hero / OG (1200×630 + 1080×1080)
- [ ] 2 Demo storyboard (6–8 frames)
- [ ] 3 Customer one-pager PDF
- [ ] 4 Landing hero mock (desktop + mobile)
- [ ] 5 Competitive comparison visual
- [ ] 6 Pilot offer one-pager
- [ ] 7 Email header + signature

**P1**
- [ ] 8 Case study template
- [ ] 9 Diagram set (HTTP / Kafka / lifecycle)
- [ ] 10 Social templates
- [ ] 11 Feature vignettes (4)
- [ ] 12 Pricing visual
- [ ] 13 Walkthrough thumbnails (4)

**P2**
- [ ] 14 Sales deck 8–10 slides
- [ ] 15 Event poster / table tent
- [ ] 16 Icon / sticker mark
- [ ] 17 Vertical one-pager

---

# Handoff prompt (paste to Claude Design / Grok Imagine)

```
You are designing marketing collateral for ContractGate (datacontractgate.com).

Positioning: Stop bad data before it hits the warehouse — semantic contracts
at ingestion, quarantine + replay, Rust sub-ms validation.

Style: Dark, technical, confident SaaS for data engineers. Use existing logo
feel; product screenshots may be referenced from the ContractGate repo
screenshots/ folder. Avoid generic purple AI-startup aesthetics.

Produce: [ITEM NUMBER AND TITLE FROM LIST]
Format/size: [FROM LIST]
Must include: [FROM LIST]
Do not include: acquisition language, patent hype, fake customer logos,
invented metrics.

Output: production-ready PNG/PDF as specified.
```

---

*Created 2026-07-15 for customer-acquisition campaign only. Update checklist as assets land in e.g. `docs/marketing/assets/`.*
