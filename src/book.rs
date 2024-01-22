use std::collections::HashMap;

use log::debug;
use rust_decimal::Decimal;

use crate::{
    directives::{AccBal, AccStatuses, CcyBal, Directive, Posting, Transaction},
    error::{BeanError, ErrorType},
};

/// Checks postings with no `Amount` and calculates the values
/// needed for the Transaction to balance.
fn complete_postings(tx: &mut Transaction) -> Vec<BeanError> {
    debug!("balancing {tx:?}");

    let mut errs: Vec<BeanError> = Vec::new();

    let mut ccy_bals: CcyBal = HashMap::new();
    let mut postings: Vec<Posting> = Vec::new();

    let mut found_empty_posting = false;
    let mut empty_posting_index = 0;

    for (i, p) in tx.postings.iter().enumerate() {
        match &p.amount {
            None => {
                if found_empty_posting {
                    let err = BeanError::new(
                        ErrorType::MultipleEmptyPostings,
                        "",
                        tx.debug.line,
                        "Found multiple empty postings for Transaction:",
                        Some(Directive::Transaction(tx.clone())),
                    );
                    errs.push(err);
                }
                empty_posting_index = i;
                found_empty_posting = true;
            }
            Some(amount) => {
                *ccy_bals.entry(amount.ccy.clone()).or_default() += amount.number;
                postings.push(p.clone())
            }
        }
    }

    if found_empty_posting {
        let account = &tx.postings[empty_posting_index].account;
        for (ccy, number) in &ccy_bals {
            let p = Posting::new(account.clone(), -number, ccy.clone());
            postings.push(p.clone())
        }
    }

    tx.postings = postings;
    errs
}

/// Checks that Transaction balances in all currencies to 0
/// MUST be run after `complete_postings`
fn check_transaction(tx: &Transaction) -> Vec<BeanError> {
    let mut errs: Vec<BeanError> = Vec::new();
    let mut ccy_bals: CcyBal = HashMap::new();
    for p in tx.postings.iter() {
        match &p.amount {
            None => {
                let err = BeanError::new(
                    ErrorType::EmptyPosting,
                    "",
                    tx.debug.line,
                    "BUG: Found empty postings after they should've been removed!",
                    Some(Directive::Transaction(tx.clone())),
                );
                errs.push(err);
            }
            Some(amount) => {
                *ccy_bals.entry(amount.ccy.clone()).or_default() += amount.number;
            }
        }
    }

    for (ccy, bal) in ccy_bals {
        // TODO get precision from context
        if bal.abs() > Decimal::new(1, 3) {
            let err = BeanError::new(
                ErrorType::UnbalancedTransaction,
                "",
                tx.debug.line,
                &format!("Transaction unbalanced for currency: {ccy}", ccy = &ccy),
                Some(Directive::Transaction(tx.clone())),
            );
            errs.push(err);
        }
    }
    errs
}

/// Complete postings as needed and check balances
/// Directives MUST be sorted appropriately before calling this
pub fn balance_transactions(directives: &mut [Directive]) -> Vec<BeanError> {
    let mut errs: Vec<BeanError> = Vec::new();
    for d in directives.iter_mut() {
        if let Directive::Transaction(tx) = d {
            errs.extend(complete_postings(tx));
            errs.extend(check_transaction(tx));
        }
    }
    errs
}

/// Get balances for all accounts in all currencies
pub fn get_balances(directives: Vec<Directive>) -> (AccBal, Vec<BeanError>) {
    let mut bals: AccBal = HashMap::new();
    let mut accs: AccStatuses = HashMap::new();
    let mut errs: Vec<BeanError> = Vec::new();
    for d in directives {
        match d {
            Directive::Open(open) => {
                accs.insert(open.account, true);
            }
            Directive::Close(close) => {
                accs.insert(close.account, false);
            }
            Directive::Balance(bal) => {
                let def = &Decimal::default();
                let entry = bals.entry(bal.account.clone()).or_default();
                let accum_bal = entry.get(&bal.amount.ccy).unwrap_or(def);
                let assert_bal = bal.amount.number;
                if (assert_bal - *accum_bal) > Decimal::new(1, 3) {
                    let err = BeanError::new(
                        ErrorType::BalanceAssertion,
                        "",
                        bal.debug.line,
                        &format!("Balance assertion failed: asserted {assert_bal} is not equal to {accum_bal}"),
                        Some(Directive::Balance(bal.clone())),
                    );
                    errs.push(err);
                }
            }
            Directive::Transaction(tx) => {
                for p in &tx.postings {
                    if let Some(amount) = &p.amount {
                        let status = accs.get(&p.account);
                        match status {
                            Some(open) => {
                                if *open {
                                    let entry = bals.entry(p.account.clone()).or_default();
                                    *entry.entry(amount.ccy.clone()).or_default() += amount.number;
                                } else {
                                    let err = BeanError::new(
                                        ErrorType::ClosedAccount,
                                        "",
                                        tx.debug.line,
                                        &format!(
                                            "Transaction referred to closed Account: {account}",
                                            account = &p.account
                                        ),
                                        Some(Directive::Transaction(tx.clone())),
                                    );
                                    errs.push(err);
                                }
                            }
                            None => {
                                let err = BeanError::new(
                                    ErrorType::NoAccount,
                                    "",
                                    tx.debug.line,
                                    &format!(
                                        "Transaction referred to non-existent Account: {account}",
                                        account = &p.account
                                    ),
                                    Some(Directive::Transaction(tx.clone())),
                                );
                                errs.push(err);
                            }
                        }
                    }
                }
            }
            _ => (),
        }
    }
    (bals, errs)
}
