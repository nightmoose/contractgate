# P5 serverless contract-enforcement pipeline.
# Does not touch ContractGate prod or the Fly apps.

.PHONY: test package fmt plan apply destroy smoke run poison status

test:
	python tests/test_handler.py

package:
	./scripts/package.sh

fmt:
	$(MAKE) -C terraform fmt

plan:
	$(MAKE) -C terraform plan

apply:
	$(MAKE) -C terraform apply

destroy:
	$(MAKE) -C terraform destroy

smoke:
	python scripts/drive.py smoke

run:
	python scripts/drive.py run --records 100000

poison:
	python scripts/drive.py poison

status:
	python scripts/drive.py status
