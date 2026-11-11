# Foundation stack for NightMoose AWS proof projects (P1).
# Commands that talk to AWS go through with-deploy-role.sh so the
# nightmoose-bots environment keys are not overwritten.
# This Makefile never targets ContractGate prod or the Fly apps.

TF      ?= terraform
TFFLAGS ?= -input=false
AWSWRAP ?= ./with-deploy-role.sh

.PHONY: fmt init validate plan apply destroy

fmt:
	$(TF) fmt -recursive

init:
	$(AWSWRAP) $(TF) init $(TFFLAGS)

validate:
	$(AWSWRAP) $(TF) validate

plan:
	$(AWSWRAP) $(TF) plan $(TFFLAGS)

# Interactive apply. Do not use this to stand up P6–P8 compute.
apply:
	$(AWSWRAP) $(TF) apply $(TFFLAGS)

# Tears down this stack only (the P2–P5 roles). Later project stacks should
# ship their own make destroy and run it the same day they apply.
destroy:
	$(AWSWRAP) $(TF) destroy $(TFFLAGS)
