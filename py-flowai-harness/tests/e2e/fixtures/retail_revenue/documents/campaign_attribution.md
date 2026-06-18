# Campaign Attribution Guide

Campaign attribution uses the campaign recorded directly on the order. Campaign
performance questions should join `orders.campaign_id` to `campaigns.id`.

Campaign attribution does not change net revenue. Apply the revenue policy
first, then group or filter by campaign if the question asks for it.
