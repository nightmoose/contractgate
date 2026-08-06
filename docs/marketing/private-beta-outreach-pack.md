# ContractGate — Private Beta Outreach Pack

**Status:** Research complete · messages ready to send  
**Constraints honored:** private 1:1 only · no public posts · no employer tagging · ODCS/Bitol people excluded  
**Product framing used:** lightweight real-time enforcement gate for data contracts on Kafka (validate at ingress → clean topic / quarantine · semantic YAML contracts · not just schema shape)

---

## What I did vs what you still need to do

### Done for you
- Deep research on all 6 named targets + 3 additional strong candidates
- Contact vectors (X, LinkedIn, GitHub, personal sites, public emails where found)
- Specific recent content anchors for personalization
- Ready-to-send short messages (DM / email style)
- Full list ranked by estimated useful-reply likelihood
- Competitive / framing notes so you don’t step on landmines

### Still on you (I cannot do these from here)
1. **Send the messages** from your real accounts (X DM, LinkedIn connection note, email).
2. **Unlock LinkedIn contact info** where needed (InMail if not connected).
3. **Confirm any private email** you already have (conference lists, mutual intros).
4. **Pick your send order** (recommendation below) and track replies in a simple sheet.
5. **Optional:** warm intro via a mutual if you have one (especially for Bellemare / Yokota / Derosiaux).
6. **Decide whether to message Gable founders (Chad / Adrian) at all** — they sell adjacent product; see notes.
7. **Decide Conduktor angle** for Stéphane — peer feedback vs perceived competitive pitch.

---

## Competitive / framing notes (read before sending)

| Person | Risk | How to frame |
|--------|------|--------------|
| **Chad Sanderson, Adrian Kreuziger** | Gable.ai is a shift-left data contracts / change-management platform | **Optional / lower priority.** Frame as complementary runtime Kafka gate (quarantine path), not “we’re also data contracts.” Or skip if you only want pure practitioners. |
| **Stéphane Derosiaux** | Conduktor has Schema Registry Proxy + Gateway (broker-boundary enforcement) | Closest product adjacency. Frame as **peer technical exchange / early beta feedback**, not “replace Conduktor.” His Dec 2025 post is the best hook. |
| **Confluent folks** (Matthew, Adam, Robert, Gunnar, Kai’s older work) | They ship Schema Registry data contracts / CEL rules | Position ContractGate as **semantic enforcement + quarantine/replay gate that sits in front of / beside SR**, not “SR is wrong.” Many teams still lack a hard choke point producers can’t route around. |
| **Ananth Packkildurai** | Practitioner / writer; built Schemata | Cleanest non-competitive practitioner outreach. |

---

## Priority ranking (highest useful-reply likelihood first)

| Rank | Name | Why this rank |
|------|------|----------------|
| 1 | **Stéphane Derosiaux** | Literally wrote “a schema is not a contract” and argues for broker-boundary enforcement — highest problem–product fit; active on X; technical peer tone matches. |
| 2 | **Adam Bellemare** | Deep, recent writing on bad data in event streams and shift-left prevention; author/practitioner DNA; highly likely to engage on technical details. |
| 3 | **Robert Yokota** *(new)* | Built Confluent Schema Registry data contracts + CEL rules + Rust SR client; most precise technical peer for enforcement mechanics. |
| 4 | **Gunnar Morling** *(new)* | Multiple conference talks on streaming data contracts with Debezium/Flink; very active on X; open-source collaborator energy. |
| 5 | **Matthew O’Keefe** | Recent InfoWorld piece on Kafka/Flink as the place to *enforce* contracts (spec / implement / enforce); strong conceptual fit. |
| 6 | **Ananth Packkildurai** *(new)* | Wrote engineering guides on Fronting Kafka / WAP validation and Schemata; pure practitioner; less vendor conflict. |
| 7 | **Kai Wähner** | Long public trail on Kafka policy enforcement + data quality; very open to tools; more high-volume field CTO — reply quality varies. |
| 8 | **Adrian Kreuziger** | Best implementation-focused co-author of engineer’s guide; now Gable CTO — high knowledge, medium competitive tension. |
| 9 | **Chad Sanderson** | Loudest data-contracts voice; CEO of Gable — high awareness, higher risk of “we already build this” or no reply to competing tools. |

**Suggested send waves**
- **Wave A (this week):** Stéphane → Adam → Robert → Gunnar  
- **Wave B (+3–5 days):** Matthew → Ananth → Kai  
- **Wave C (optional):** Adrian → Chad (only if you want ecosystem dialogue, not pure beta users)

---

# Full person dossiers + ready-to-send messages

---

## 1. Stéphane Derosiaux

**Role / company:** Co-founder & CPTO (Chief Product & Technology Officer), **Conduktor**  
**Best contact methods (priority order):**
1. **X/Twitter DM:** [@sderosiaux](https://x.com/sderosiaux) — active, technical
2. LinkedIn: [stephane-derosiaux](https://www.linkedin.com/in/stephane-derosiaux)
3. GitHub: [sderosiaux](https://github.com/sderosiaux)
4. Company path: via Conduktor contact / mutual Kafka community (last resort)

**Relevant recent content**
- [Kafka Data Contracts: A Schema Is Not a Contract](https://www.conduktor.io/blog/kafka-data-contracts) (Dec 23, 2025) — core thesis matches ContractGate
- [Stop Calling Your Kafka Topics Data Products](https://www.conduktor.io/blog/kafka-data-products) (Feb 3, 2026) — “Schema-on-a-wiki is not a contract”
- [Schema Evolution: 8 Kafka Best Practices](https://www.conduktor.io/glossary/schema-evolution-best-practices) (Jul 2026)

**Openness signals:** Writes bluntly about enforcement gaps; active public technical writing; builder/CTO posture.

**Follow-up if no reply:** 7–10 days, one short bump; then stop.

### Ready-to-send message (X DM or email style)

```
Hi Stéphane — your Dec piece “A Schema Is Not a Contract” is the cleanest write-up I’ve seen of the gap: registries check shape at registration, but producers can still ship semantically broken events past the boundary.

I’ve been building ContractGate privately: a lightweight real-time enforcement gate for Kafka. Producers write to a raw topic; we validate against a semantic YAML contract (types, patterns, enums, required fields, glossary rules) and route to clean vs quarantine. Sub-ms per event, Rust core, optional Kafka Connect path.

Very early private beta — looking for a handful of people who live this problem, not publicity. Would you take a 15-minute screen-share comparing your current enforcement path to the gate, or should I just send a private invite link?

No pressure either way — happy to keep it peer-to-peer technical.
```

---

## 2. Adam Bellemare

**Role / company:** Principal Technologist, Technology Strategy Group / Office of the CTO, **Confluent**  
**Formerly:** data platform engineer at Shopify, Flipp, BlackBerry  
**Author:** *Building Event-Driven Microservices*, *Building an Event-Driven Data Mesh* (O’Reilly)

**Best contact methods:**
1. LinkedIn: [adambellemare](https://www.linkedin.com/in/adambellemare) — primary (strong professional presence)
2. GitHub: [bellemare](https://github.com/bellemare)
3. X: historically referenced as @AdamBellemare; activity unclear — prefer LinkedIn unless you confirm active X
4. Warm intro via Confluent/Kafka Summit network if available

**Relevant recent content**
- [Shift Left: Bad Data in Event Streams, Part 1](https://www.confluent.io/blog/shift-left-bad-data-in-event-streams-part-1/) (Oct 2024) — prevention first on immutable logs
- [Data Products, Data Contracts, and Change Data Capture](https://www.confluent.io/blog/implementing-streaming-data-products/) (Feb 2024)
- Ongoing writing on streaming data products / event design

**Openness signals:** Writes long, practical technical posts; engages practitioner problems (fixing bad data, contracts as product boundaries).

**Follow-up if no reply:** 10–14 days (busy author/speaker cadence).

### Ready-to-send message

```
Hi Adam — your “Shift Left: Bad Data in Event Streams” series nails the hard constraint: once bad events hit an immutable log, batch-style fix-ups don’t transfer. Prevention has to be the first strategy.

I’m building ContractGate in private: a small real-time gate in front of Kafka topics that enforces semantic data contracts at ingress (not just schema ID / shape) and quarantines violators so consumers never see them. YAML contracts, Rust validation engine, clean vs quarantine routing.

Not looking for press — just a few practitioners who care about producer–consumer agreements on streams. Would you take 15 minutes to screen-share how you currently prevent bad events, vs a quick look at the gate? Or I can send a private invite only.

Totally fine if timing is bad — no hard sell.
```

---

## 3. Robert Yokota *(additional candidate)*

**Role / company:** Staff Software Engineer (data governance / Schema Registry), **Confluent**  
**Why included:** Primary implementer of Confluent Schema Registry **data contracts** (quality rules, CEL, migration rules, CSFLE integration). Deepest technical peer on enforcement mechanics.

**Best contact methods:**
1. X: [@rayokota](https://x.com/rayokota)
2. Personal blog comments / presence: [yokota.blog](https://yokota.blog/)
3. GitHub: [rayokota](https://github.com/rayokota) (incl. [rust-schema-registry-client](https://github.com/rayokota/rust-schema-registry-client))
4. LinkedIn: [robert-yokota-477108](https://www.linkedin.com/in/robert-yokota-477108)

**Relevant recent content**
- [Using Data Contracts with the Rust Schema Registry Client](https://yokota.blog/2025/04/16/using-data-contracts-with-the-rust-schema-registry-client/) (Apr 2025)
- [How to Protect PII in Kafka With Schema Registry and Data Contracts](https://www.confluent.io/blog/protect-pii-kafka-data-contracts/) (Aug 2025)
- [JSON Schema Compatibility and the Robustness Principle](https://yokota.blog/2025/10/07/json-schema-compatibility-and-the-robustness-principle/) (Oct 2025)
- Original: [Using Data Contracts with Confluent Schema Registry](https://www.confluent.io/blog/data-contracts-confluent-schema-registry/)

**Openness signals:** Publishes detailed implementation blogs; maintains open-source SR clients; engages schema/contract edge cases.

**Follow-up if no reply:** 10–14 days.

### Ready-to-send message

```
Hi Robert — I’ve been reading your work on SR data contracts (CEL quality rules, migration rules, and the Rust client). The design of contracts as structure + integrity constraints + evolution is the right mental model.

I’ve been building something adjacent in private: ContractGate — a real-time semantic enforcement gate for Kafka that sits as an ingress path (raw → validate → clean/quarantine). Aim is the cases where client-side SR rules aren’t opted into, or teams want a hard boundary producers can’t skip, plus quarantine/replay for bad events.

Very early private beta; looking for a few people who actually implement this layer — not a marketing round. Open to a 15-minute technical screen-share of your current enforcement path vs the gate, or just a private invite if you’d rather poke at it alone.

Either way, happy to keep it engineer-to-engineer.
```

---

## 4. Gunnar Morling *(additional candidate)*

**Role / company:** Technologist, **Confluent** (ex-Debezium lead at Red Hat; formerly Decodable)  
**Best contact methods:**
1. **X/Twitter DM:** [@gunnarmorling](https://x.com/gunnarmorling) — very active, high engagement
2. Personal site: [morling.dev](https://www.morling.dev/)
3. LinkedIn: gunnar-morling
4. Conference hallway / GitHub for OSS context

**Relevant recent content**
- Talk series: **Data Contracts In Practice With Debezium and Apache Flink** (Kafka Summit London 2024, Current 2024, Flink Forward, etc.) — [session](https://www.confluent.io/events/kafka-summit-london-2024/data-contracts-in-practice-with-debezium-and-apache-flink/) · [slides](https://speakerdeck.com/gunnarmorling/data-contracts-in-practice-with-debezium-and-apache-flink)
- CDC / stream contract themes on [morling.dev](https://www.morling.dev/)

**Openness signals:** Explicitly invites hallway chat at conferences; highly active on X; OSS-first personality.

**Follow-up if no reply:** 7–10 days on X.

### Ready-to-send message

```
Hi Gunnar — your “Data Contracts in Practice with Debezium and Flink” talk is one of the few that treats contracts as something you actually enforce on CDC streams, not a wiki page.

I’m privately building ContractGate: a lightweight Kafka ingress gate that validates events against semantic contracts and routes bad ones to quarantine before consumers see them. Useful when the producer path is heterogeneous (CDC + apps + connectors) and you need one choke point.

Early private beta — looking for a few real users who care about streaming contracts, not publicity. Open to a 15-minute screen-share of a typical Debezium→Kafka contract path vs the gate, or I can send a private invite.

No pressure — purely technical peer outreach.
```

---

## 5. Matthew O’Keefe

**Role / company:** Principal Technologist, Technology Strategy Group, **Confluent** (data modeling, schema discovery/management, shift-left). Still publishing under Confluent as of 2026 (InfoWorld Dec 2025; Confluent blog Jan 2026). X bio: Principal Technologist @ Confluent.

**Best contact methods:**
1. **X:** [@matthewokeefe1](https://x.com/matthewokeefe1)
2. LinkedIn: [matthewtokeefe](https://www.linkedin.com/in/matthewtokeefe)
3. Article author pages (InfoWorld / Confluent) — no public personal email found

**Relevant recent content**
- [Why data contracts need Apache Kafka and Apache Flink](https://www.infoworld.com/article/4086004/why-data-contracts-need-apache-kafka-and-apache-flink.html) (Dec 2, 2025) — **spec / implement / enforce**; enforcement is the missing leg
- [Streaming Data Integration with Apache Kafka vs ETL](https://www.confluent.io/blog/streaming-data-integration-with-kafka-vs-etl/) (Jan 29, 2026)
- LinkedIn series: Data Modeling to Enable Shift Left (Parts I–II, 2025)

**Openness signals:** Regular external writing; entrepreneur background (founded companies); discusses practical enforcement requirements.

**Follow-up if no reply:** 10–14 days.

### Ready-to-send message

```
Hi Matthew — your InfoWorld piece on why data contracts need Kafka/Flink is the clearest split I’ve seen: specification, implementation, and enforcement as three distinct requirements. Most teams stop at “we have a schema in the registry.”

I’m building ContractGate privately as an enforcement gate: real-time validation of semantic contracts on Kafka ingress, with clean vs quarantine routing so bad events don’t poison consumers. Complements registry-based structure with a hard path-level check producers can’t quietly skip.

Early private beta — looking for a handful of people who care about modeling + enforcement, not a launch. Would a 15-minute screen-share of “current flow vs gate” be useful, or prefer a private invite to poke at yourself?

Low pressure either way.
```

---

## 6. Ananth Packkildurai *(additional candidate)*

**Role / company:** Principal Architect / Principal Engineer, **Zeta Global** (agentic AI + data platforms); ex-Slack, Zendesk  
**Also:** Author of **Data Engineering Weekly**; creator of **Schemata** (decentralized schema/data contracts framework)

**Best contact methods:**
1. **X:** [@ananthdurai](https://x.com/ananthdurai)
2. Substack: [Data Engineering Weekly](https://www.dataengineeringweekly.com/) / [@dataengineeringweekly](https://substack.com/@dataengineeringweekly) — Message on Substack
3. LinkedIn: [ananthdurai](https://www.linkedin.com/in/ananthdurai)
4. GitHub: [ananthdurai](https://github.com/ananthdurai) (Schemata)

**Relevant recent content**
- [An Engineering Guide to Data Quality — Data Contract Perspective (Part 2)](https://www.dataengineeringweekly.com/p/an-engineering-guide-to-data-quality) — Fronting Kafka / WAP validation pattern; notes gap in real-time event-routing tools
- [Introducing Schemata](https://www.dataengineeringweekly.com/p/introducing-schemata-a-decentralized)
- Ongoing DE Weekly curation (still active 2026)

**Openness signals:** Built open-source contract tooling; wrote about building an event-routing validator (“Nobu”); newsletter/Substack messaging is natural.

**Follow-up if no reply:** 7–10 days via X or Substack.

### Ready-to-send message

```
Hi Ananth — your data-contract quality series (especially the Fronting Kafka / WAP patterns) is still one of the best engineering treatments of “validate before publish” on streams. The point that real-time event routing still lacks first-class contract tools stuck with me.

I’ve been privately building ContractGate: a Rust-based gate that validates semantic contracts at Kafka ingress and routes to clean vs quarantine topics. Aimed at the one-phase choke-point case — not another observability dashboard.

Very early private beta; looking for practitioners, not coverage. Would you take 15 minutes to compare notes on fronting-Kafka validation, or should I send a private invite only?

Happy to keep it purely technical.
```

---

## 7. Kai Wähner (Kai Waehner)

**Role / company:** Advisory Field CTO, **Kai Waehner GmbH**; Global Field CTO at **Kestra** (ex-Confluent Field CTO; also Talend, TIBCO)

**Best contact methods:**
1. **Email (public):** [contact@kai-waehner.de](mailto:contact@kai-waehner.de) — listed on his site for contact / collaboration
2. Also seen: kontakt@kai-waehner.de (privacy page)
3. X: [@KaiWaehner](https://x.com/KaiWaehner)
4. LinkedIn: [kaiwaehner](https://www.linkedin.com/in/kaiwaehner/)
5. Site: [kai-waehner.de](https://www.kai-waehner.de/)

**Relevant recent content**
- [Policy Enforcement and Data Quality for Apache Kafka with Schema Registry](https://www.kai-waehner.de/blog/2023/10/16/data-quality-and-policy-enforcement-for-apache-kafka-with-schema-registry/) — data contracts / field-level rules / DLQ patterns
- Continues publishing landscapes (streaming, integration, agentic AI) through 2026; shift-left architecture posts still reference data contracts on Kafka topics

**Openness signals:** Explicitly invites collaboration when a real use case is worth covering; high volume of tool/vendor landscape writing — good for awareness, less “I’ll beta-test your binary.”

**Follow-up if no reply:** 14 days once; don’t spam.

### Ready-to-send message

```
Hi Kai — your post on policy enforcement and data quality for Kafka with Schema Registry is still the practical map of “structure is not enough — you need field-level rules and a place to put bad messages.”

I’ve been building ContractGate privately: a lightweight real-time enforcement gate (semantic YAML contracts, clean vs quarantine routing) for teams that want a hard ingress boundary on Kafka, not only registry registration checks.

Early private beta — seeking a few real users and technical feedback, not publicity or a landscape placement. If useful, I’d value 15 minutes comparing a typical SR+rules setup to the gate, or I can send a private invite.

Happy to keep this off any public channels.
```

---

## 8. Adrian Kreuziger

**Role / company:** Co-founder & **CTO, Gable.ai** (raised Series A); ex-Principal Engineer, data platform at **Convoy**

**Best contact methods:**
1. LinkedIn: [adrian-kreuziger-b737b115b](https://www.linkedin.com/in/adrian-kreuziger-b737b115b)
2. Via Gable / co-author content with Chad (no strong public X found)
3. Substack presence historically via Chad’s Data Products posts

**Relevant recent content**
- Co-author: [An Engineer’s Guide to Data Contracts – Pt. 1](https://dataproducts.substack.com/p/an-engineers-guide-to-data-contracts) — enforcement at producer, Schema Registry compatibility, CI checks
- Implementation-focused Convoy-era patterns still cited widely
- Gable: shift-left data platform (code lineage, contracts in the developer workflow)

**Openness signals:** Practitioner-turned-founder; raised capital on this problem — **high knowledge, competitive product adjacency**.

**Follow-up if no reply:** one bump at 14 days; then stop. Prefer complementary framing only.

### Ready-to-send message (complementary / careful)

```
Hi Adrian — your Engineer’s Guide (with Chad) is still the implementation blueprint I send people: contracts as producer-enforced agreements with SR compatibility + CI gates.

I’m privately building a narrow piece adjacent to that stack: ContractGate — real-time semantic validation at Kafka ingress with quarantine/replay for events that already escaped CI. Not trying to replace shift-left change management; focused on the runtime choke point for streaming paths.

Early private beta; looking for a few people who’ve actually implemented contracts in production for feedback. If a 15-minute peer screen-share is interesting (your current runtime path vs the gate), great; if it’s too close to Gable’s world, totally understand — no hard feelings.

Keeping this private either way.
```

---

## 9. Chad Sanderson

**Role / company:** Co-founder & **CEO, Gable.ai**; author of forthcoming O’Reilly *Data Contracts* (with Mark Freeman); Substack **Data Products**

**Best contact methods:**
1. Substack: [dataproducts.substack.com](https://dataproducts.substack.com/) / [@chadsanderson](https://substack.com/@chadsanderson) — Message
2. LinkedIn: [chad-sanderson](https://www.linkedin.com/in/chad-sanderson) — large audience; use connection note carefully (private, not public post)
3. X: [@ChadSanderson](https://x.com/ChadSanderson) — appears low-activity; LinkedIn/Substack better
4. Avoid cold “salesy” Gable demo channels

**Relevant recent content**
- [The Consumer-Defined Data Contract](https://dataproducts.substack.com/p/the-consumer-defined-data-contract)
- Ongoing LinkedIn writing on adoption failures and shift-left
- Gable positioning: contracts + change management in the producer code path

**Openness signals:** Extremely public on contracts; **founder of competing/adjacent category**. Best treated as ecosystem conversation, not core beta user.

**Follow-up if no reply:** one polite bump at 14–21 days; do not pursue.

### Ready-to-send message (optional / ecosystem)

```
Hi Chad — long-time reader of Data Products. Your consumer-defined contract framing and the adoption failure modes are the honest version of this space.

I’ve been privately building a narrow runtime piece: ContractGate — a Kafka ingress gate for semantic contract enforcement with quarantine, for teams that already buy the “contracts as APIs” idea but still get bad events into topics when producers skip client-side checks.

Not pitching a broad platform or publicity. If you ever want a 15-minute look as a peer in the contracts space (or to tell me it’s redundant with paths you already see), I’m game. Otherwise no need to reply.

Keeping this 1:1.
```

---

# Quick-reference contact table

| Person | Primary | Secondary | Email (public) |
|--------|---------|-----------|----------------|
| Stéphane Derosiaux | X @sderosiaux | LinkedIn | — (conduktor.io corporate only) |
| Adam Bellemare | LinkedIn adambellemare | GitHub bellemare | — |
| Robert Yokota | X @rayokota | yokota.blog / GitHub rayokota | — |
| Gunnar Morling | X @gunnarmorling | morling.dev | — |
| Matthew O’Keefe | X @matthewokeefe1 | LinkedIn matthewtokeefe | — |
| Ananth Packkildurai | X @ananthdurai | Substack DE Weekly | — |
| Kai Wähner | email contact@kai-waehner.de | X @KaiWaehner | **contact@kai-waehner.de** |
| Adrian Kreuziger | LinkedIn | — | — |
| Chad Sanderson | Substack / LinkedIn | X @ChadSanderson | — |

---

# Tracking sheet template (copy)

| Name | Channel | Sent date | Follow-up date | Reply? | Notes |
|------|---------|-----------|----------------|--------|-------|
| Stéphane Derosiaux | | | | | |
| Adam Bellemare | | | | | |
| Robert Yokota | | | | | |
| Gunnar Morling | | | | | |
| Matthew O’Keefe | | | | | |
| Ananth Packkildurai | | | | | |
| Kai Wähner | | | | | |
| Adrian Kreuziger | | | | | |
| Chad Sanderson | | | | | |

---

# Assets to have ready before they say yes

1. **Private invite path** — cloud account or self-hosted `make demo` link with 5-minute script  
2. **15-min screen-share outline**  
   - Their current flow (2 min)  
   - ContractGate: contract → produce bad event → quarantine (5 min)  
   - Ask: would this fit / what’s missing (5 min)  
   - Next step or not (3 min)  
3. **One-pager** (internal only) on Kafka ingress + quarantine topics  
4. **Clear ask** — “use for 2 weeks on one non-prod topic and tell me what breaks”

---

# Research exclusions (per your constraints)

Not contacted / not added: Jean-Georges Perrin, Simon Harrer, Jochen Christ, Peter Flook, Andrew Jones, Andy Petrella, Martin Meermeyer, Andrea Gioia, Dirk Van de Poel, Gene Stakhov, Atanas Iliev, and other primarily ODCS/Bitol/TSC-associated people.
