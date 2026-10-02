"""Deepline GTM API via stable-deepline.dev (x402, USDC, no API key)."""

from __future__ import annotations

import sys

from .runner import ToolSpec, serve
from .schema import arr, i, obj, s

BASE_URL = "https://stable-deepline.dev"

TOOLS = [
    ToolSpec(
        name="deepline_email_work",
        description=(
            "Find a verified work email. $0.04. Waterfall across 79+ providers (Apollo, "
            "Hunter, Prospeo, ...) from first/last name + company domain; stops at the "
            "first hit. Optional company_name and linkedin_url hints improve coverage."
        ),
        method="POST",
        path="/api/email/work",
        has_body=True,
        input_schema=obj(
            {
                "first_name": s("Contact first name."),
                "last_name": s("Contact last name."),
                "domain": s("Company domain, e.g. acme.com."),
                "company_name": s("Optional company name hint."),
                "linkedin_url": s(
                    "Optional standard LinkedIn profile URL; enables LinkedIn-native fallback providers."
                ),
            },
            required=["first_name", "last_name", "domain"],
        ),
    ),
    ToolSpec(
        name="deepline_email_from_linkedin",
        description=(
            "Resolve a LinkedIn profile URL into a verified work email. $0.17. "
            "Waterfall search (Findymail, Prospeo, Lusha, ...)."
        ),
        method="POST",
        path="/api/email/from-linkedin",
        has_body=True,
        input_schema=obj(
            {"linkedin_url": s("Standard LinkedIn profile URL in /in/ format.")},
            required=["linkedin_url"],
        ),
    ),
    ToolSpec(
        name="deepline_email_personal",
        description=(
            "Find a personal email. $0.10. Waterfall (Dropleads, LeadMagic, PeopleDataLabs). "
            "Best coverage with a LinkedIn URL plus first and last name."
        ),
        method="POST",
        path="/api/email/personal",
        has_body=True,
        input_schema=obj(
            {
                "first_name": s("Contact first name."),
                "last_name": s("Contact last name."),
                "linkedin_url": s("LinkedIn profile URL; improves coverage."),
                "domain": s("Company domain context."),
                "company_name": s("Company name context."),
            },
            required=["first_name", "last_name"],
        ),
    ),
    ToolSpec(
        name="deepline_email_validate",
        description=(
            "Validate an email address via ZeroBounce deliverability check. $0.03. "
            "Result is valid / catch-all / invalid."
        ),
        method="POST",
        path="/api/email/validate",
        has_body=True,
        input_schema=obj(
            {
                "email": s("Email address to validate."),
                "ip_address": s("Optional signup IP address used by ZeroBounce for enrichment."),
            },
            required=["email"],
        ),
    ),
    ToolSpec(
        name="deepline_person_from_email",
        description=(
            "Reverse an email into the person behind it: name, title, company, LinkedIn URL. "
            "$0.12. Provide an email (work addresses; personal webmail is rejected) and/or "
            "a linkedin_url."
        ),
        method="POST",
        path="/api/person/from-email",
        has_body=True,
        input_schema=obj(
            {
                "email": s("Business/work email address to enrich. Personal webmail domains are rejected."),
                "linkedin_url": s("LinkedIn profile URL to enrich."),
            }
        ),
    ),
    ToolSpec(
        name="deepline_person_job_change",
        description=(
            "Check whether a contact changed jobs vs. the company you have on record. $0.15. "
            "Compares the LinkedIn profile against current_domain."
        ),
        method="POST",
        path="/api/person/job-change",
        has_body=True,
        input_schema=obj(
            {
                "linkedin_url": s("LinkedIn profile URL (/in/ format) of the contact."),
                "current_domain": s("Bare domain of the company on record, used to detect a change."),
            },
            required=["linkedin_url"],
        ),
    ),
    ToolSpec(
        name="deepline_linkedin_find",
        description=(
            "Find a person's LinkedIn profile URL from name plus company context. $0.10. "
            "Provide first/last name and at least one of domain, company_name, or email."
        ),
        method="POST",
        path="/api/linkedin/find",
        has_body=True,
        input_schema=obj(
            {
                "first_name": s("Contact first name."),
                "last_name": s("Contact last name."),
                "domain": s("Company domain context."),
                "company_name": s("Company name context."),
                "email": s("Optional known email context."),
            },
            required=["first_name", "last_name"],
        ),
    ),
    ToolSpec(
        name="deepline_phone_find",
        description=(
            "Find a validated mobile phone number. $0.50. Automatic region-based provider "
            "routing; strongest coverage in US/CA and Western Europe. Provide first/last "
            "name plus any context (domain, email, linkedin_url)."
        ),
        method="POST",
        path="/api/phone/find",
        has_body=True,
        input_schema=obj(
            {
                "first_name": s("Contact first name."),
                "last_name": s("Contact last name."),
                "domain": s("Company domain context."),
                "email": s("Optional known email context."),
                "linkedin_url": s("LinkedIn profile URL; improves coverage."),
            },
            required=["first_name", "last_name"],
        ),
    ),
    ToolSpec(
        name="deepline_company_enrich",
        description=(
            "Enrich a company with a merged 30+ field profile (Apollo, Crustdata, "
            "PeopleDataLabs): revenue, headcount, funding, valuation, tech stack, socials, "
            "SIC/NAICS codes, HQ address. $0.10. Identify by domain, company_name, or linkedin."
        ),
        method="POST",
        path="/api/company/enrich",
        has_body=True,
        input_schema=obj(
            {
                "domain": s("Company domain to enrich, e.g. stripe.com."),
                "company_name": s("Company name fallback when domain is unavailable."),
                "linkedin": s("LinkedIn company page id or full company URL."),
            }
        ),
    ),
    ToolSpec(
        name="deepline_contacts_by_role",
        description=(
            "Find decision makers at a company by role and optional seniority, accumulated "
            "across Dropleads, Apollo, Icypeas, Prospeo, and Crustdata. $0.25. "
            "Identify the company by domain (preferred), company_name, or linkedin_company_url."
        ),
        method="POST",
        path="/api/contacts/by-role",
        has_body=True,
        input_schema=obj(
            {
                "roles": arr("string", 'Persona intents, e.g. ["VP Engineering", "Head of Security"].', minItems=1),
                "company_name": s("Company name for context and provider fallbacks."),
                "domain": s("Company domain for lookup, e.g. acme.com."),
                "linkedin_company_url": s("Optional LinkedIn company URL (/company/ format) for domain-less lookup."),
                "seniority": arr(
                    "string",
                    'Seniority intent, e.g. ["C-Level", "VP", "Director", "Manager", "Senior"].',
                ),
                "limit": i(
                    "Optional result cap per waterfall step 1-100. Use 1 for the cheapest lookups.",
                    minimum=1,
                    maximum=100,
                ),
            },
            required=["roles"],
        ),
    ),
    ToolSpec(
        name="deepline_ads_search",
        description=(
            "Ad intelligence: a company's active ads on Facebook, Google, LinkedIn, or "
            "TikTok (via Adyntel): creative copy, spend signals, targeting. $0.02. "
            "domain is required for facebook/google/linkedin; keyword is required for tiktok."
        ),
        method="POST",
        path="/api/ads/search",
        has_body=True,
        input_schema=obj(
            {
                "platform": s("Ad platform to query.", enum=["facebook", "google", "linkedin", "tiktok"]),
                "domain": s("Normalized company domain (no https/www). Required for facebook, google, linkedin."),
                "keyword": s("Keyword for the TikTok ad library. Required for tiktok (brand name works)."),
                "media_type": s("Google ad format filter (google only).", enum=["text", "image", "video"]),
                "country_code": s('TikTok country filter: a European country code or "ALL". Omit for global.'),
            },
            required=["platform"],
        ),
    ),
]


def main(argv: list[str] | None = None) -> None:
    serve(
        "x402 Deepline GTM",
        "0.1.0",
        TOOLS,
        default_base_url=BASE_URL,
        default_timeout=120.0,
        argv=argv,
    )


if __name__ == "__main__":
    main(sys.argv[1:])
