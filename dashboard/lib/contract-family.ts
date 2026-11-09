/**
 * Labels for a contract family's pipeline boundary.
 *
 * `stage` is the between-stages boundary: the contract checks the payload
 * after a transform and before the next one (raw → staging, staging → mart).
 */

export function boundaryLabel(boundary: string): string {
  switch (boundary) {
    case "ingest":
      return "Ingest";
    case "stage":
      return "Between stages";
    case "egress":
      return "Egress";
    default:
      return boundary;
  }
}
