"""People Data Labs via stablepeopledata.dev (x402, USDC, no API key)."""

from __future__ import annotations

import sys

from .runner import ToolSpec, serve
from .schema import arr, b, i, obj, s

BASE_URL = "https://stablepeopledata.dev"

_COMMON_ENRICH = {
    "min_likelihood": i(
        "Minimum confidence score 1-10 (default 2). Higher = fewer, better matches.",
        minimum=1,
        maximum=10,
    ),
    "required": s("Boolean expression of fields that must be present in the response."),
    "data_include": s("Comma-separated fields to include in response; prefix with - to exclude."),
    "include_if_matched": b("Include the list of matched query inputs."),
    "titlecase": b("Titlecase text in the response."),
}

_SEARCH_PARAMS = {
    "query": {
        "type": "object",
        "description": "Elasticsearch v7.7 DSL query object.",
        "additionalProperties": True,
    },
    "sql": s("SQL query, e.g. SELECT * FROM person WHERE ... Do not include LIMIT; use size."),
    "size": i("Batch size 1-100 (default 1). Price scales with size.", minimum=1, maximum=100),
    "scroll_token": s("Pagination token from a previous response."),
    "titlecase": b("Titlecase text in the response."),
}

TOOLS = [
    ToolSpec(
        name="pdl_person_enrich",
        description=(
            "People Data Labs person enrichment. $0.28 per match, free on no match. "
            "Returns demographics, work history, education, contact info, social profiles, "
            "skills. Provide at least one identifier: pdl_id, email, phone, profile URL "
            "(e.g. LinkedIn), name parts, or company/school + location."
        ),
        method="POST",
        path="/api/pdl/person/enrich",
        has_body=True,
        input_schema=obj(
            {
                "pdl_id": s("PDL persistent person ID."),
                "name": s("Full name (first + last)."),
                "first_name": s("First name."),
                "last_name": s("Last name."),
                "middle_name": s("Middle name."),
                "location": s("Residence location (street address down to country)."),
                "street_address": s("Street address."),
                "locality": s("City."),
                "region": s("State or province."),
                "country": s("Country."),
                "postal_code": s("ZIP or postal code."),
                "company": s("Company name, website, or social URL."),
                "school": s("School name, website, or social URL."),
                "phone": s("Phone number, must start with +country_code."),
                "email": s("Email address."),
                "email_hash": s("SHA-256 or MD5 email hash."),
                "profile": s("Social profile URL (e.g. LinkedIn /in/ URL)."),
                "lid": s("LinkedIn numeric ID."),
                "birth_date": s("Birth date, YYYY or YYYY-MM-DD."),
                **_COMMON_ENRICH,
            }
        ),
    ),
    ToolSpec(
        name="pdl_company_enrich",
        description=(
            "People Data Labs company enrichment. $0.10 per match, free on no match. "
            "Returns firmographics, industry codes, funding, social profiles. Provide at "
            "least one identifier: pdl_id (or LinkedIn slug), name, website domain, "
            "ticker, profile URL, or HQ location."
        ),
        method="POST",
        path="/api/pdl/company/enrich",
        has_body=True,
        input_schema=obj(
            {
                "pdl_id": s("PDL company ID or LinkedIn slug."),
                "name": s("Company name."),
                "website": s("Company domain, e.g. stripe.com."),
                "profile": s("Social profile URL."),
                "ticker": s("Stock ticker symbol."),
                "location": s("HQ location (street address down to country)."),
                "street_address": s("HQ street address."),
                "locality": s("HQ city."),
                "region": s("HQ state or province."),
                "country": s("HQ country."),
                "postal_code": s("HQ postal code."),
                **_COMMON_ENRICH,
            }
        ),
    ),
    ToolSpec(
        name="pdl_person_search",
        description=(
            "Search 3B+ People Data Labs person records via Elasticsearch DSL or SQL. "
            "$0.28 x size (max $28); free when the result set is empty. Recommended size 1-5. "
            "Paginate with the returned scroll_token. dataset picks record sources "
            "(resume, email, phone, mobile_phone, street_address, consumer_social, developer, all)."
        ),
        method="POST",
        path="/api/pdl/person/search",
        has_body=True,
        input_schema=obj(
            {
                **_SEARCH_PARAMS,
                "dataset": s(
                    "Datasets to include: resume, email, phone, mobile_phone, street_address, "
                    "consumer_social, developer, all."
                ),
            }
        ),
    ),
    ToolSpec(
        name="pdl_company_search",
        description=(
            "Search 30M+ People Data Labs company records via Elasticsearch DSL or SQL. "
            "$0.10 x size (max $10); free when the result set is empty. Recommended size 1-5. "
            "Paginate with the returned scroll_token."
        ),
        method="POST",
        path="/api/pdl/company/search",
        has_body=True,
        input_schema=obj(dict(_SEARCH_PARAMS)),
    ),
]


def main(argv: list[str] | None = None) -> None:
    serve(
        "x402 People Data Labs",
        "0.1.0",
        TOOLS,
        default_base_url=BASE_URL,
        default_timeout=60.0,
        argv=argv,
    )


if __name__ == "__main__":
    main(sys.argv[1:])
