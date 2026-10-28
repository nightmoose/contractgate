# LangChain, LlamaIndex, and Instructor

AISchemaGate checks a finished JSON object with `POST /egress/{contract_id}`.
A contract name is resolved with `GET /contracts`. A UUID is sent as the
path id. The Python adapters below call the SDK's existing egress helper.
They return the egress payload (declared transforms already applied). They
do not accept a token stream.

```bash
pip install 'aischemagate[langchain]'
pip install 'aischemagate[llamaindex]'
pip install 'aischemagate[instructor]'
```

`import aischemagate` does not load these modules. The API host is
`https://api.datacontractgate.com`. The key is the same `cg_live_...` key as
DataContractGate.

## LangChain

```python
import os
from langchain_core.prompts import ChatPromptTemplate
from aischemagate.langchain import AISchemaGateParser

parser = AISchemaGateParser(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
)
prompt = ChatPromptTemplate.from_template("Extract {topic} as JSON.")
# `llm` is a LangChain chat model. The extra installs `langchain-core`.
chain = prompt | llm | parser
payload = chain.invoke({"topic": "the invoice"})
```

`parse` raises `OutputParserException` when the text is not a JSON object or
the contract rejects it.

## LlamaIndex

```python
import os
from llama_index.core.prompts import PromptTemplate
from aischemagate.llamaindex import AISchemaGateParser

parser = AISchemaGateParser(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
)
prompt = PromptTemplate("Extract the invoice as JSON.", output_parser=parser)
query = prompt.format()
payload = parser.parse(response_text)
```

A rejection raises `AISchemaGateRejected`.

## Instructor

Guard the response model. Instructor validates that class; the guard runs
the structural check before Pydantic field validation. A structural failure
is the validation error, and field validators do not run. A passing check
hands Pydantic the egress payload.

```python
import os
import instructor
from openai import OpenAI
from pydantic import BaseModel
from aischemagate.instructor import AISchemaGateHook

hook = AISchemaGateHook(
    api_key=os.environ["CONTRACTGATE_API_KEY"],
    contract_id=os.environ["AISG_CONTRACT_ID"],
)

@hook.guard
class Invoice(BaseModel):
    total: float
    currency: str

llm = instructor.from_openai(OpenAI())
invoice = llm.chat.completions.create(
    model="gpt-4o-mini",
    response_model=Invoice,
    messages=[{"role": "user", "content": "Extract the invoice as JSON."}],
)
```
