// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use diem_events_fetcher::DiemEventsFetcher;
use diem_sdk::{client::views::EventDataView, transaction_builder::Currency};
use forge::{PublicUsageContext, PublicUsageTest, Result, Test};

pub struct EventFetcher;

impl Test for EventFetcher {
    fn name(&self) -> &'static str {
        "smoke-test::event-fetcher"
    }
}

impl PublicUsageTest for EventFetcher {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let client = ctx.client();
        let factory = ctx.transaction_factory();

        let mut account1 = ctx.random_account();
        ctx.create_parent_vasp_account(account1.authentication_key())?;
        ctx.fund(account1.address(), 100_000_000)?;

        let account2 = ctx.random_account();
        ctx.create_parent_vasp_account(account2.authentication_key())?;
        ctx.fund(account2.address(), 100_000_000)?;

        let txn = account1.sign_with_transaction_builder(factory.peer_to_peer(
            Currency::XUS,
            account2.address(),
            3_000_000,
        ));
        client.submit(&txn)?;
        client.wait_for_signed_transaction(&txn, None, None)?;

        let txn = account1.sign_with_transaction_builder(factory.peer_to_peer(
            Currency::XUS,
            account2.address(),
            4_000_000,
        ));
        client.submit(&txn)?;
        client.wait_for_signed_transaction(&txn, None, None)?;

        let events_fetcher = DiemEventsFetcher::new(ctx.url())?;

        runtime.block_on(async move {
            let (sent_handle, received_handle) = events_fetcher
                .get_payment_event_handles(account1.address())
                .await
                .unwrap()
                .unwrap();
            let mut sent_events = events_fetcher.get_all_events(&sent_handle).await.unwrap();
            let received_events = events_fetcher
                .get_all_events(&received_handle)
                .await
                .unwrap();

            assert_eq!(received_events.len(), 1);
            assert_eq!(received_handle.count(), 1);
            assert_eq!(sent_handle.count(), 2);
            assert_eq!(sent_events.len(), 2);
            let evt1 = sent_events.pop().unwrap();
            let evt2 = sent_events.pop().unwrap();

            if let EventDataView::SentPayment { amount, .. } = evt1.data {
                assert_eq!(amount.amount, 4000000);
            } else {
                panic!("invalid event");
            }

            if let EventDataView::SentPayment { amount, .. } = evt2.data {
                assert_eq!(amount.amount, 3000000);
            } else {
                panic!("invalid event");
            }
        });

        Ok(())
    }
}
