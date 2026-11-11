# P3: RAG over ContractGate docs with Bedrock Knowledge Bases + S3 Vectors.
# AWS commands that create the stack go through the P1 wrapper so the
# nightmoose-bots env keys are not overwritten. This Makefile never
# targets ContractGate prod or the Fly apps, and it does not touch
# infra/aws/terraform.tfstate.

TF      ?= terraform
TFDIR   ?= terraform
TFFLAGS ?= -input=false
AWSWRAP ?= ../../infra/aws/with-deploy-role.sh
export NMB_TF_SESSION ?= terraform-p3
PYTHON  ?= python3

.PHONY: fmt init validate plan apply destroy collect ingest eval ask serve

fmt:
	$(TF) -chdir=$(TFDIR) fmt -recursive

init:
	NMB_TF_SESSION=$(NMB_TF_SESSION) $(AWSWRAP) $(TF) -chdir=$(TFDIR) init $(TFFLAGS)

validate:
	NMB_TF_SESSION=$(NMB_TF_SESSION) $(AWSWRAP) $(TF) -chdir=$(TFDIR) validate

plan:
	NMB_TF_SESSION=$(NMB_TF_SESSION) $(AWSWRAP) $(TF) -chdir=$(TFDIR) plan $(TFFLAGS)

apply:
	NMB_TF_SESSION=$(NMB_TF_SESSION) $(AWSWRAP) $(TF) -chdir=$(TFDIR) apply $(TFFLAGS) -auto-approve

# Tears down this stack only (vector bucket, indexes, KBs, data sources,
# grounding guardrail). Does not delete nmb-bedrock-kb or anything in P1.
destroy:
	NMB_TF_SESSION=$(NMB_TF_SESSION) $(AWSWRAP) $(TF) -chdir=$(TFDIR) destroy $(TFFLAGS) -auto-approve

collect:
	$(PYTHON) scripts/collect_corpus.py

ingest: collect
	$(PYTHON) scripts/ingest.py

eval:
	$(PYTHON) scripts/eval_rag.py

ask:
	$(PYTHON) scripts/ask.py $(Q)

serve:
	$(PYTHON) scripts/serve.py
