/** Keep upgrade copy on the current host.
 *
 * The API may still return the production pricing URL in `upgrade_url` and
 * in the 402 message so existing API clients stay backward compatible. The
 * dashboard link is always `/pricing`.
 */
export function localUpgradeMessage(message: string): string {
  return message.replace(/https:\/\/app\.datacontractgate\.com\/pricing\S*/g, "/pricing");
}
