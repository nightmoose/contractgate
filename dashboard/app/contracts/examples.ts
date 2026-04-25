/**
 * Starter content for the contract editor and the JSON sample box on the
 * `/contracts` page.  Lifted out of `page.tsx` so a 1100-line component
 * doesn't carry inline 50-line fixtures, and so the YAML schema stays
 * findable by anyone touching the on-boarding example.
 *
 * Behavioral note: these strings are user-editable defaults, not server
 * authority.  The Rust backend re-validates whatever the user submits.
 */

export const EXAMPLE_YAML = `version: "1.0"
name: "my_events"
description: "Replace this with your contract"

ontology:
  entities:
    - name: user_id
      type: string
      required: true
      pattern: "^[a-zA-Z0-9_-]{3,64}$"

    - name: event_type
      type: string
      required: true
      enum:
        - "click"
        - "view"
        - "purchase"

    - name: timestamp
      type: integer
      required: true
      min: 0

    - name: amount
      type: number
      required: false
      min: 0

glossary:
  - field: "user_id"
    description: "Unique user identifier"
  - field: "amount"
    description: "Monetary value in USD"
    constraints: "must be non-negative"

metrics:
  - name: "total_revenue"
    formula: "sum(amount) where event_type = 'purchase'"
`;

export const EXAMPLE_SAMPLE = `[
  { "user_id": "alice_01", "event_type": "click", "timestamp": 1712000001, "page": "/home" },
  { "user_id": "bob_99",   "event_type": "purchase", "timestamp": 1712000002, "amount": 49.99, "page": "/checkout" },
  { "user_id": "carol_x",  "event_type": "login",  "timestamp": 1712000003 },
  { "user_id": "dave_7",   "event_type": "view",   "timestamp": 1712000004, "amount": 0, "page": "/product" },
  { "user_id": "eve_22",   "event_type": "click",  "timestamp": 1712000005, "page": "/about" }
]`;
