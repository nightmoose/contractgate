# P5 serverless pipeline stack. AWS commands go through with-deploy-role.sh
# so the nightmoose-bots environment keys are not overwritten.
# This Makefile never targets ContractGate prod or the Fly apps.

TF      ?= terraform
TFFLAGS ?= -input=false
AWSWRAP ?= ./with-deploy-role.sh
PKG     ?= ../scripts/package.sh

.PHONY: fmt package init validate plan apply destroy

fmt:
	$(TF) fmt -recursive

package:
	$(PKG)

init: package
	$(AWSWRAP) $(TF) init $(TFFLAGS)

validate: package
	$(AWSWRAP) $(TF) validate

plan: package
	$(AWSWRAP) $(TF) plan $(TFFLAGS)

apply: package
	$(AWSWRAP) $(TF) apply $(TFFLAGS) -auto-approve

# Tears down this stack only (Lambda, SFN, SQS, DynamoDB, EventBridge,
# dashboard, alarm, log groups). Does not delete the /nmb/ IAM roles.
destroy:
	$(AWSWRAP) $(TF) destroy $(TFFLAGS) -auto-approve
