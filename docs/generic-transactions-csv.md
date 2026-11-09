# Generic transactions CSV

Open column set for a bank, ledger, or bookkeeping export that arrives as a
spreadsheet or CSV. DataContractGate authors this shape. It is not a vendor
schema, and it is not a claim that DataContractGate is affiliated with itself.

The file syntax follows [RFC 4180](https://www.rfc-editor.org/rfc/rfc4180)
(comma-separated, header row, quoted fields when needed). The columns are:

| Column | Required | Meaning |
|---|---|---|
| `date` | yes | Transaction date, `YYYY-MM-DD` |
| `description` | yes | Payee or memo |
| `amount` | yes | Signed number. Negative is a debit, positive is a credit |
| `currency` | yes | Uppercase ISO 4217 code |
| `category` | no | Bookkeeping category |
| `account` | no | Account name |
| `reference` | no | Check number, transfer id, or other reference |
| `balance` | no | Running balance after the row, when the export includes one |

The catalog contract is `catalog/seeds/generic-sheets-transactions-csv/`.
Its "Verified against" link points at RFC 4180, the CSV syntax this file
format follows. Samples in that directory are synthetic.
