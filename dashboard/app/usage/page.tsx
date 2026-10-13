// RFC-104: `/usage` is an API endpoint on the Rust backend (see
// `src/main.rs:1424`), not a dashboard page. Visitors who type the URL
// expecting a usage dashboard were landing on a 404. The usage widget already
// lives on `/account` — redirect there instead.
import { redirect } from "next/navigation";

export default function UsagePage(): never {
  redirect("/account");
}
