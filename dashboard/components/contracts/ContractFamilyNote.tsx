"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { familyForContract, type ContractFamily } from "@/lib/api";
import { boundaryLabel } from "@/lib/contract-family";

/** Shown on a contract when it belongs to a private family. */
export default function ContractFamilyNote({ contractId }: { contractId: string }) {
  const [family, setFamily] = useState<ContractFamily | null>(null);

  useEffect(() => {
    let cancelled = false;
    familyForContract(contractId).then((found) => {
      if (!cancelled) setFamily(found);
    });
    return () => {
      cancelled = true;
    };
  }, [contractId]);

  if (!family) return null;
  const link = family.links.find((row) => row.contract_id === contractId);
  return (
    <p className="text-xs text-slate-400 mt-2">
      Part of{" "}
      <Link
        href={`/contracts/families/${family.id}`}
        className="text-teal-300 hover:text-teal-200"
      >
        {family.name}
      </Link>
      {link ? (
        <span className="text-slate-500">
          {" "}
          · {boundaryLabel(link.boundary)} · step {link.position + 1} of {family.links.length}
        </span>
      ) : null}
    </p>
  );
}
